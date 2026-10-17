use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use crate::model::{NifBlock, NifFile, NifValue};
use crate::sf_mesh::SfMeshData;
use crate::skeleton_repose::{
    BindMatrix, local_of, mat4_affine_inverse, mat4_mul, node_order_and_parents, set_local,
};
use crate::skin::bone_remap::{SkeletonMap, VertexInfluences};

const HAVOK_SCALE: f64 = 69.99125;

pub(crate) struct SkinContext {
    map: SkeletonMap,
    bones: HashMap<String, (String, usize, BindMatrix)>,
    cloth_bone_names: HashMap<String, String>,
    companion_binds: HashMap<String, Vec<BindMatrix>>,
    root: usize,
    pub cloth_blobs: u32,
    pub custom_cloth_bones: u32,
    pub unbound_cloth_bones: u32,
}

#[derive(Debug)]
pub(crate) struct SkinnedGeometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub influences: Vec<VertexInfluences>,
    pub instance: usize,
    pub reduced_vertices: u32,
    pub max_discarded_weight: f32,
}

impl SkinContext {
    pub fn load(
        dest: &mut NifFile,
        skeleton: &Path,
        maps: &Path,
        source: &NifFile,
        companions: &[PathBuf],
    ) -> Result<Self, String> {
        let map = SkeletonMap::load(maps, "starfield", "fo4").map_err(|e| e.to_string())?;
        let skeleton = NifFile::load(skeleton.to_path_buf()).map_err(|e| e.to_string())?;
        let mut context = Self::install(dest, &skeleton, map)?;
        for block in &source.blocks {
            if block.type_name != "BSClothExtraData" {
                continue;
            }
            let data = block
                .get_field("Binary Data")
                .ok_or("Starfield cloth block has no binary data")?;
            let blob = crate::cloth::byte_array_to_bytes(data).map_err(|e| e.to_string())?;
            let adapted = crate::sf_cloth::adapt_starfield_cloth(&blob, &skeleton, &context.map)?;
            let validation =
                havok_native::api::cloth_validate(&adapted.blob).map_err(|e| e.to_string())?;
            let validation: serde_json::Value =
                serde_json::from_str(&validation).map_err(|e| e.to_string())?;
            if validation["valid"] != true {
                return Err(format!(
                    "Starfield cloth failed target validation: {validation}"
                ));
            }
            context.install_cloth_bones(dest, adapted.bones)?;
            context.unbound_cloth_bones += adapted.unbound_unused.len() as u32;
            let id = dest.add_block("BSClothExtraData", None);
            dest.blocks[id].set_field(
                "Name",
                block
                    .get_field("Name")
                    .cloned()
                    .unwrap_or_else(|| NifValue::String("CES".into())),
            );
            dest.blocks[id].set_field(
                "Binary Data",
                crate::cloth::bytes_to_byte_array(&adapted.blob),
            );
            let root = &mut dest.blocks[context.root];
            let mut extras = match root.get_field("Extra Data List") {
                Some(NifValue::Array(values)) => values.clone(),
                _ => Vec::new(),
            };
            extras.push(NifValue::Ref(id as i32));
            root.set_field("Num Extra Data List", NifValue::UInt(extras.len() as u64));
            root.set_field("Extra Data List", NifValue::Array(extras));
            context.cloth_blobs += 1;
        }
        let needed = source_skin_binds(source)?
            .into_keys()
            .filter(|name| {
                context.map.lookup(name).is_none() && !context.cloth_bone_names.contains_key(name)
            })
            .collect::<HashSet<_>>();
        for path in companions {
            let companion = NifFile::load(path.clone())
                .map_err(|error| format!("cloth companion {}: {error}", path.display()))?;
            let owner_binds = source_skin_binds(&companion)?;
            let mut supplied = HashSet::new();
            for block in companion
                .blocks
                .iter()
                .filter(|block| block.type_name == "BSClothExtraData")
            {
                let blob = crate::cloth::byte_array_to_bytes(
                    block
                        .get_field("Binary Data")
                        .ok_or("cloth companion has no binary data")?,
                )
                .map_err(|error| error.to_string())?;
                let adapted =
                    crate::sf_cloth::adapt_starfield_cloth(&blob, &skeleton, &context.map)?;
                let validation = havok_native::api::cloth_validate(&adapted.blob)
                    .map_err(|error| error.to_string())?;
                let validation: serde_json::Value =
                    serde_json::from_str(&validation).map_err(|error| error.to_string())?;
                if validation["valid"] != true {
                    return Err(format!(
                        "cloth companion failed target validation: {validation}"
                    ));
                }
                let mut targets = HashSet::new();
                for binding in &adapted.bones {
                    let name = binding.source_name.to_ascii_lowercase();
                    if needed.contains(&name) {
                        let binds = owner_binds.get(&name).ok_or_else(|| {
                            format!(
                                "cloth companion has no skin bind for {}",
                                binding.source_name
                            )
                        })?;
                        context
                            .companion_binds
                            .entry(name.clone())
                            .or_default()
                            .extend(binds.iter().cloned());
                        supplied.insert(name);
                        targets.insert(binding.target_name.to_ascii_lowercase());
                    }
                }
                loop {
                    let parents: Vec<_> = adapted
                        .bones
                        .iter()
                        .filter(|binding| {
                            targets.contains(&binding.target_name.to_ascii_lowercase())
                        })
                        .filter_map(|binding| {
                            binding
                                .custom
                                .as_ref()
                                .map(|custom| custom.parent.to_ascii_lowercase())
                        })
                        .collect();
                    let mut changed = false;
                    for parent in parents {
                        changed |= targets.insert(parent);
                    }
                    if !changed {
                        break;
                    }
                }
                context.install_cloth_bones(
                    dest,
                    adapted
                        .bones
                        .into_iter()
                        .filter(|binding| {
                            targets.contains(&binding.target_name.to_ascii_lowercase())
                        })
                        .collect(),
                )?;
            }
            if supplied.is_empty() {
                return Err(format!(
                    "cloth companion {} supplies no required skin bones",
                    path.display()
                ));
            }
        }
        Ok(context)
    }

    fn install(dest: &mut NifFile, skeleton: &NifFile, map: SkeletonMap) -> Result<Self, String> {
        let (order, parents) = node_order_and_parents(skeleton);
        let mut copied = HashMap::new();
        let mut worlds = HashMap::new();
        let mut bones = HashMap::new();
        let root = dest.header.footer_roots.first().copied().unwrap_or(0) as usize;
        for source_id in order {
            let source = &skeleton.blocks[source_id];
            let parent_id = match parents.get(&source_id) {
                Some(parent) => *copied
                    .get(parent)
                    .ok_or("target skeleton has a parent cycle")?,
                None => root,
            };
            let local = scaled_local(source)?;
            let world = match parents.get(&source_id) {
                Some(parent) => mat4_mul(
                    worlds.get(parent).ok_or("missing target parent pose")?,
                    &local,
                ),
                None => local,
            };
            let mut fields = IndexMap::new();
            for key in ["Name", "Translation", "Rotation", "Scale"] {
                if let Some(value) = source.get_field(key) {
                    fields.insert(key.to_string(), value.clone());
                }
            }
            fields.insert("Flags".into(), NifValue::UInt(14));
            let dest_id = dest.add_block("NiNode", Some(fields));
            attach(dest, parent_id, dest_id);
            copied.insert(source_id, dest_id);
            worlds.insert(source_id, world);
            if let Some(NifValue::String(name)) = source.get_field("Name") {
                let name = name.trim_end_matches('\0').to_string();
                if bones
                    .insert(name.to_ascii_lowercase(), (name, dest_id, world))
                    .is_some()
                {
                    return Err("target skeleton contains duplicate bone names".into());
                }
            }
        }
        if bones.is_empty() {
            return Err("target skeleton has no named bones".into());
        }
        Ok(Self {
            map,
            bones,
            cloth_bone_names: HashMap::new(),
            companion_binds: HashMap::new(),
            root,
            cloth_blobs: 0,
            custom_cloth_bones: 0,
            unbound_cloth_bones: 0,
        })
    }

    fn install_cloth_bones(
        &mut self,
        dest: &mut NifFile,
        bindings: Vec<crate::sf_cloth::ClothBoneBinding>,
    ) -> Result<(), String> {
        for binding in bindings {
            let key = binding.target_name.to_ascii_lowercase();
            if let Some((_, _, world)) = self.bones.get(&key) {
                if world
                    .iter()
                    .flatten()
                    .zip(binding.world.iter().flatten())
                    .any(|(a, b)| (a - b).abs() > 1e-3)
                {
                    return Err(format!(
                        "Starfield cloth has conflicting rest poses for {}",
                        binding.target_name
                    ));
                }
            } else if let Some(custom) = binding.custom {
                let (_, parent, parent_world) = self
                    .bones
                    .get(&custom.parent.to_ascii_lowercase())
                    .ok_or_else(|| {
                        format!(
                            "Starfield cloth custom bone {} has no parent {}",
                            binding.target_name, custom.parent
                        )
                    })?;
                let expected_world = mat4_mul(parent_world, &custom.local);
                if expected_world
                    .iter()
                    .flatten()
                    .zip(binding.world.iter().flatten())
                    .any(|(a, b)| (a - b).abs() > 1e-3)
                {
                    return Err(format!(
                        "Starfield cloth custom bone {} has an inconsistent rest pose",
                        binding.target_name
                    ));
                }
                let parent = *parent;
                let id = dest.add_block("NiNode", None);
                dest.blocks[id].set_field("Name", NifValue::String(binding.target_name.clone()));
                dest.blocks[id].set_field("Flags", NifValue::UInt(14));
                set_local(&mut dest.blocks[id], &custom.local);
                attach(dest, parent, id);
                self.bones
                    .insert(key, (binding.target_name.clone(), id, binding.world));
                self.custom_cloth_bones += 1;
            } else {
                return Err(format!(
                    "Starfield cloth target bone {} is missing",
                    binding.target_name
                ));
            }
            let source = binding.source_name.to_ascii_lowercase();
            if let Some(previous) = self
                .cloth_bone_names
                .insert(source, binding.target_name.clone())
            {
                if previous != binding.target_name {
                    return Err(format!(
                        "Starfield cloth has conflicting mappings for {}",
                        binding.source_name
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn convert(
        &self,
        source: &NifFile,
        shape: &NifBlock,
        mesh: &SfMeshData,
        dest: &mut NifFile,
    ) -> Result<SkinnedGeometry, String> {
        let weights = mesh
            .bone_influences
            .as_ref()
            .ok_or("skin has no mesh weights")?;
        if weights.len() != mesh.positions.len() {
            return Err("skin weight count does not match vertices".into());
        }
        let instance = source
            .get_block(
                reference(shape.get_field("Skin"))
                    .ok_or("weighted Starfield geometry has no skin instance")?,
            )
            .ok_or("missing Starfield skin instance")?;
        let data = source
            .get_block(
                reference(instance.get_field("Data")).ok_or("Starfield skin has no bone data")?,
            )
            .ok_or("missing Starfield bone data")?;
        let names = source_bone_names(source, shape, instance)?;
        let Some(NifValue::Array(binds)) = data.get_field("Bone List") else {
            return Err("Starfield skin has no inverse binds".into());
        };
        if names.len() != binds.len() || names.is_empty() {
            return Err("Starfield bone names and inverse binds disagree".into());
        }

        let mut target_refs = Vec::new();
        let mut target_binds = Vec::new();
        let mut target_indices = HashMap::new();
        let mut source_to_target = Vec::new();
        let mut repose = Vec::new();
        let mut used = vec![false; names.len()];
        for influence in weights {
            for &(index, weight) in &influence.slots {
                if !weight.is_finite() || weight < 0.0 {
                    return Err("invalid skin weight".into());
                }
                if weight > 0.0 {
                    *used
                        .get_mut(index)
                        .ok_or("mesh bone index exceeds skin palette")? = true;
                }
            }
        }
        for (source_index, (name, bind)) in names.iter().zip(binds).enumerate() {
            if !used[source_index] {
                source_to_target.push(None);
                repose.push(None);
                continue;
            }
            if let Some(owner_binds) = self.companion_binds.get(&name.to_ascii_lowercase()) {
                if !owner_binds.contains(&source_bind_matrix(shape, bind)?) {
                    return Err(format!("cloth companion inverse bind disagrees for {name}"));
                }
            }
            let target_name = self
                .cloth_bone_names
                .get(&name.to_ascii_lowercase())
                .map(String::as_str)
                .or_else(|| self.map.lookup(name))
                .ok_or_else(|| format!("unmapped Starfield skin bone {name}"))?;
            let (_, target_ref, target_world) = self
                .bones
                .get(&target_name.to_ascii_lowercase())
                .ok_or_else(|| {
                format!("target skeleton is missing {target_name} for {name}")
            })?;
            let target_index = *target_indices.entry(*target_ref).or_insert_with(|| {
                let index = target_refs.len();
                target_refs.push(*target_ref);
                target_binds.push(mat4_affine_inverse(target_world));
                index
            });
            if target_binds[target_index].is_none() || target_index > 255 {
                return Err("target skin has a singular bind or more than 256 bones".into());
            }
            source_to_target.push(Some(target_index));
            let NifValue::Struct(fields) = bind else {
                return Err("invalid Starfield inverse bind".into());
            };
            let mut block = shape.clone();
            block.fields = fields.clone();
            let mut bind = scaled_local(&block)?;
            // Starfield inverse binds consume raw shape-local metres. Geometry world
            // transforms are already in those binds and must not be baked a second time.
            for row in &mut bind[..3] {
                for value in row {
                    *value *= HAVOK_SCALE;
                }
            }
            repose.push(Some(mat4_mul(target_world, &bind)));
        }

        let mut output = SkinnedGeometry {
            positions: Vec::with_capacity(weights.len()),
            normals: Vec::with_capacity(weights.len()),
            influences: Vec::with_capacity(weights.len()),
            instance: 0,
            reduced_vertices: 0,
            max_discarded_weight: 0.0,
        };
        for (vertex, influence) in weights.iter().enumerate() {
            let total: f32 = influence.slots.iter().map(|(_, w)| *w).sum();
            if !total.is_finite() || total <= 0.0 {
                return Err(format!(
                    "Starfield vertex {vertex} has no valid skin weights"
                ));
            }
            let mut position = [0.0; 3];
            let mut normal = [0.0; 3];
            let mut merged: Vec<(usize, f32)> = Vec::new();
            for &(source_index, weight) in &influence.slots {
                if weight == 0.0 {
                    continue;
                }
                let transform = repose
                    .get(source_index)
                    .and_then(Option::as_ref)
                    .ok_or("mesh bone index exceeds skin palette")?;
                let target = source_to_target[source_index].ok_or("missing used skin bone")?;
                let weight = weight / total;
                let p = transform_point(transform, mesh.positions[vertex]);
                let n = transform_normal(
                    transform,
                    mesh.normals.get(vertex).copied().unwrap_or([0.0, 0.0, 1.0]),
                )
                .map_err(|error| {
                    format!(
                        "{error} at vertex {vertex}, source normal {:?}, bone {}",
                        mesh.normals.get(vertex),
                        names[source_index]
                    )
                })?;
                for axis in 0..3 {
                    position[axis] += p[axis] * weight;
                    normal[axis] += n[axis] * weight;
                }
                match merged.iter_mut().find(|(bone, _)| *bone == target) {
                    Some((_, value)) => *value += weight,
                    None => merged.push((target, weight)),
                }
            }
            merged.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            let discarded: f32 = merged.iter().skip(4).map(|(_, weight)| *weight).sum();
            if discarded > 0.0 {
                output.reduced_vertices += 1;
            }
            output.max_discarded_weight = output.max_discarded_weight.max(discarded);
            merged.truncate(4);
            let retained: f32 = merged.iter().map(|(_, weight)| *weight).sum();
            for (_, weight) in &mut merged {
                *weight /= retained;
            }
            output.positions.push(position);
            output.normals.push(
                normalize(normal).map_err(|error| {
                    format!("{error} after blending vertex {vertex}: {normal:?}")
                })?,
            );
            output.influences.push(VertexInfluences { slots: merged });
        }

        let mut bone_list = Vec::new();
        for (index, bind) in target_binds.into_iter().enumerate() {
            let bind = bind.ok_or("singular target bind")?;
            let points = output
                .positions
                .iter()
                .zip(&output.influences)
                .filter(|(_, inf)| {
                    inf.slots
                        .iter()
                        .any(|(bone, weight)| *bone == index && *weight > 0.0)
                })
                .map(|(point, _)| transform_point(&bind, *point))
                .collect::<Vec<_>>();
            let mut center = [0.0; 3];
            if !points.is_empty() {
                for axis in 0..3 {
                    let lo = points.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min);
                    let hi = points
                        .iter()
                        .map(|p| p[axis])
                        .fold(f32::NEG_INFINITY, f32::max);
                    center[axis] = (lo + hi) * 0.5;
                }
            }
            let radius = points
                .iter()
                .map(|p| {
                    (0..3)
                        .map(|a| (p[a] - center[a]).powi(2))
                        .sum::<f32>()
                        .sqrt()
                })
                .fold(0.0, f32::max);
            let mut sphere = IndexMap::new();
            sphere.insert("Center".into(), NifValue::Vec3(center));
            sphere.insert("Radius".into(), NifValue::Float(radius as f64));
            let mut block = shape.clone();
            block.fields = IndexMap::new();
            set_local(&mut block, &bind);
            block.set_field("Scale", NifValue::Float(1.0));
            block.set_field("Bounding Sphere", NifValue::Struct(sphere));
            bone_list.push(NifValue::Struct(block.fields));
        }
        let mut data = IndexMap::new();
        data.insert("Num Bones".into(), NifValue::UInt(bone_list.len() as u64));
        data.insert("Bone List".into(), NifValue::Array(bone_list));
        let data = dest.add_block("BSSkin::BoneData", Some(data));
        let mut instance = IndexMap::new();
        instance.insert("Skeleton Root".into(), NifValue::Ref(self.root as i32));
        instance.insert("Data".into(), NifValue::Ref(data as i32));
        instance.insert("Num Bones".into(), NifValue::UInt(target_refs.len() as u64));
        instance.insert(
            "Bones".into(),
            NifValue::Array(
                target_refs
                    .into_iter()
                    .map(|id| NifValue::Ref(id as i32))
                    .collect(),
            ),
        );
        instance.insert("Num Scales".into(), NifValue::UInt(0));
        instance.insert("Scales".into(), NifValue::Array(Vec::new()));
        output.instance = dest.add_block("BSSkin::Instance", Some(instance));
        Ok(output)
    }
}

fn scaled_local(block: &NifBlock) -> Result<BindMatrix, String> {
    let mut local = local_of(block);
    let scale = match block.get_field("Scale") {
        Some(NifValue::Float(s)) => *s,
        Some(NifValue::UInt(s)) => *s as f64,
        Some(NifValue::Int(s)) => *s as f64,
        _ => 1.0,
    };
    for row in &mut local[..3] {
        for v in &mut row[..3] {
            *v *= scale;
        }
    }
    if !local.iter().flatten().all(|v| v.is_finite()) || mat4_affine_inverse(&local).is_none() {
        return Err("nonfinite or singular skin transform".into());
    }
    Ok(local)
}

fn source_bind_matrix(shape: &NifBlock, bind: &NifValue) -> Result<BindMatrix, String> {
    let NifValue::Struct(fields) = bind else {
        return Err("invalid Starfield inverse bind".into());
    };
    let mut block = shape.clone();
    block.fields = fields.clone();
    scaled_local(&block)
}

fn source_skin_binds(source: &NifFile) -> Result<HashMap<String, Vec<BindMatrix>>, String> {
    let mut result: HashMap<String, Vec<BindMatrix>> = HashMap::new();
    for shape in &source.blocks {
        let Some(instance) = reference(shape.get_field("Skin")).and_then(|id| source.get_block(id))
        else {
            continue;
        };
        let data = reference(instance.get_field("Data"))
            .and_then(|id| source.get_block(id))
            .ok_or("Starfield skin has no bone data")?;
        let names = source_bone_names(source, shape, instance)?;
        let Some(NifValue::Array(binds)) = data.get_field("Bone List") else {
            return Err("Starfield skin has no inverse binds".into());
        };
        if names.len() != binds.len() {
            return Err("Starfield bone names and inverse binds disagree".into());
        }
        for (name, bind) in names.iter().zip(binds) {
            result
                .entry(name.to_ascii_lowercase())
                .or_default()
                .push(source_bind_matrix(shape, bind)?);
        }
    }
    Ok(result)
}

fn source_bone_names(
    source: &NifFile,
    shape: &NifBlock,
    instance: &NifBlock,
) -> Result<Vec<String>, String> {
    if let Some(NifValue::Array(extras)) = shape.get_field("Extra Data List") {
        for extra in extras {
            let Some(block) = reference(Some(extra)).and_then(|id| source.get_block(id)) else {
                continue;
            };
            if block.type_name == "SkinAttach" {
                let Some(NifValue::Array(names)) = block.get_field("Bones") else {
                    return Err("SkinAttach has no Bones table".into());
                };
                return names
                    .iter()
                    .map(|name| match name {
                        NifValue::String(name) => Ok(name.trim_end_matches('\0').to_string()),
                        _ => Err("SkinAttach has a nonstring bone name".into()),
                    })
                    .collect();
            }
        }
    }
    let Some(NifValue::Array(refs)) = instance.get_field("Bones") else {
        return Err("Starfield skin has no bone-name table".into());
    };
    refs.iter()
        .map(|value| {
            let block = reference(Some(value))
                .and_then(|id| source.get_block(id))
                .ok_or("Starfield skin has null bones and no SkinAttach")?;
            match block.get_field("Name") {
                Some(NifValue::String(name)) => Ok(name.trim_end_matches('\0').to_string()),
                _ => Err("Starfield skin bone has no name".into()),
            }
        })
        .collect()
}

fn reference(value: Option<&NifValue>) -> Option<usize> {
    match value {
        Some(NifValue::Ref(id)) if *id >= 0 => Some(*id as usize),
        _ => None,
    }
}

fn attach(nif: &mut NifFile, parent: usize, child: usize) {
    let block = &mut nif.blocks[parent];
    let mut children = match block.get_field("Children") {
        Some(NifValue::Array(values)) => values.clone(),
        _ => Vec::new(),
    };
    children.push(NifValue::Ref(child as i32));
    block.set_field("Num Children", NifValue::UInt(children.len() as u64));
    block.set_field("Children", NifValue::Array(children));
}

fn transform_point(matrix: &BindMatrix, point: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| {
        (matrix[r][3] + (0..3).map(|c| matrix[r][c] * point[c] as f64).sum::<f64>()) as f32
    })
}

fn transform_normal(matrix: &BindMatrix, normal: [f32; 3]) -> Result<[f32; 3], String> {
    let inverse = mat4_affine_inverse(matrix).ok_or("singular skin normal transform")?;
    normalize(std::array::from_fn(|r| {
        (0..3)
            .map(|c| inverse[c][r] * normal[c] as f64)
            .sum::<f64>() as f32
    }))
}

fn normalize(vector: [f32; 3]) -> Result<[f32; 3], String> {
    let length = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !length.is_finite() || length < 1e-12 {
        return Err("invalid skinned normal".into());
    }
    Ok(vector.map(|v| v / length))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(nif: &mut NifFile, name: &str, translation: [f32; 3]) -> usize {
        let id = nif.add_block("NiNode", None);
        nif.blocks[id].set_field("Name", NifValue::String(name.into()));
        nif.blocks[id].set_field("Translation", NifValue::Vec3(translation));
        id
    }

    fn fixture() -> (NifFile, usize, SfMeshData, NifFile, SkinContext) {
        let mut source = NifFile::new("starfield");
        let shape = source.add_block("BSGeometry", None);
        source.blocks[shape].set_field("Translation", NifValue::Vec3([0.0, 0.0, 9.0]));
        let mut names = IndexMap::new();
        names.insert(
            "Bones".into(),
            NifValue::Array(
                (0..6)
                    .map(|i| NifValue::String(format!("Source{i}")))
                    .collect(),
            ),
        );
        let names = source.add_block("SkinAttach", Some(names));
        source.blocks[shape].set_field(
            "Extra Data List",
            NifValue::Array(vec![NifValue::Ref(names as i32)]),
        );
        let mut bind = IndexMap::new();
        bind.insert("Translation".into(), NifValue::Vec3([0.0, 0.0, 2.0]));
        let mut data = IndexMap::new();
        data.insert(
            "Bone List".into(),
            NifValue::Array(vec![NifValue::Struct(bind); 6]),
        );
        let data = source.add_block("BSSkin::BoneData", Some(data));
        let mut instance = IndexMap::new();
        instance.insert("Data".into(), NifValue::Ref(data as i32));
        instance.insert("Bones".into(), NifValue::Array(vec![NifValue::Ref(-1); 6]));
        let instance = source.add_block("BSSkin::Instance", Some(instance));
        source.blocks[shape].set_field("Skin", NifValue::Ref(instance as i32));
        let mesh = SfMeshData {
            positions: vec![[0.0, 0.0, 1.0]],
            normals: vec![[0.0, 0.0, 1.0]],
            uvs: vec![[0.0, 0.0]],
            triangles: Vec::new(),
            vertex_colors: None,
            bone_influences: Some(vec![VertexInfluences {
                slots: vec![(0, 0.2), (1, 0.2), (2, 0.2), (3, 0.2), (4, 0.1), (5, 0.1)],
            }]),
        };
        let mut skeleton = NifFile::new("fo4");
        for i in 0..5 {
            let bone = node(&mut skeleton, &format!("Target{i}"), [0.0, 0.0, 10.0]);
            attach(&mut skeleton, 0, bone);
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skeleton_starfield_to_fo4.yaml"),
            "bones:\n  Source0: Target0\n  Source1: Target0\n  Source2: Target1\n  Source3: Target2\n  Source4: Target3\n  Source5: Target4\n").unwrap();
        let map = SkeletonMap::load(dir.path(), "starfield", "fo4").unwrap();
        let mut dest = NifFile::new("fo4");
        let context = SkinContext::install(&mut dest, &skeleton, map).unwrap();
        (source, shape, mesh, dest, context)
    }

    #[test]
    fn null_bone_refs_use_skinattach_and_raw_local_bind_once() {
        let (source, shape, mesh, mut dest, context) = fixture();
        let result = context
            .convert(&source, &source.blocks[shape], &mesh, &mut dest)
            .unwrap();
        assert!((result.positions[0][2] - (10.0 + 3.0 * HAVOK_SCALE as f32)).abs() < 0.0001);
        assert_eq!(result.normals[0], [0.0, 0.0, 1.0]);
        assert_eq!(result.influences[0].slots.len(), 4);
        assert!((result.influences[0].slots[0].1 - 0.4 / 0.9).abs() < 1e-6);
        assert_eq!(result.reduced_vertices, 1);
        assert!((result.max_discarded_weight - 0.1).abs() < 1e-6);
        assert!(
            (result.influences[0]
                .slots
                .iter()
                .map(|(_, w)| w)
                .sum::<f32>()
                - 1.0)
                .abs()
                < 1e-6
        );
        let instance = &dest.blocks[result.instance];
        let data = reference(instance.get_field("Data")).unwrap();
        let NifValue::Array(binds) = dest.blocks[data].get_field("Bone List").unwrap() else {
            panic!()
        };
        for bind in binds {
            let NifValue::Struct(fields) = bind else {
                panic!()
            };
            let mut block = source.blocks[shape].clone();
            block.fields = fields.clone();
            let p = transform_point(&scaled_local(&block).unwrap(), [0.0, 0.0, 10.0]);
            assert_eq!(p, [0.0; 3]);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skin.nif");
        dest.save(Some(path.clone())).unwrap();
        let reloaded = NifFile::load(path).unwrap();
        assert_eq!(reloaded.find_blocks("BSSkin::Instance").len(), 1);
        assert_eq!(reloaded.find_blocks("BSSkin::BoneData").len(), 1);
    }

    #[test]
    fn invalid_palette_and_missing_names_fail_instead_of_emitting_rigid_geometry() {
        let (mut source, shape, mut mesh, mut dest, context) = fixture();
        mesh.bone_influences.as_mut().unwrap()[0].slots = vec![(6, 1.0)];
        assert!(
            context
                .convert(&source, &source.blocks[shape], &mesh, &mut dest)
                .unwrap_err()
                .contains("exceeds skin palette")
        );
        mesh.bone_influences.as_mut().unwrap()[0].slots = vec![(0, 1.0)];
        source.blocks[shape].set_field("Extra Data List", NifValue::Array(Vec::new()));
        assert!(
            context
                .convert(&source, &source.blocks[shape], &mesh, &mut dest)
                .unwrap_err()
                .contains("null bones")
        );
    }

    #[test]
    fn unused_palette_entries_need_no_target_bone() {
        let (mut source, shape, mut mesh, mut dest, context) = fixture();
        let names_id = reference(
            match source.blocks[shape].get_field("Extra Data List").unwrap() {
                NifValue::Array(extras) => extras.first(),
                _ => panic!(),
            },
        )
        .unwrap();
        let NifValue::Array(names) = source.blocks[names_id].get_field_mut("Bones").unwrap() else {
            panic!()
        };
        names[5] = NifValue::String("UnusedWithoutTarget".into());
        mesh.bone_influences.as_mut().unwrap()[0].slots = vec![(0, 1.0), (5, 0.0)];
        let output = context
            .convert(&source, &source.blocks[shape], &mesh, &mut dest)
            .unwrap();
        assert_eq!(output.influences[0].slots, vec![(0, 1.0)]);
        mesh.bone_influences.as_mut().unwrap()[0].slots = vec![(5, 1.0)];
        assert!(
            context
                .convert(&source, &source.blocks[shape], &mesh, &mut dest)
                .unwrap_err()
                .contains("unmapped Starfield skin bone")
        );
    }

    #[test]
    #[ignore = "requires extracted Starfield and FO4 assets"]
    fn real_constellation_legs_preserve_cloth_and_bound_simulation_bone() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = repo.join("extracted/starfield/meshes/Clothes/Spacesuit_Constellation/Spacesuit_Constellation_Legs_M.nif");
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("legs.nif");
        let options = crate::sf_convert::SfConvertOptions {
            geometries_root: repo.join("extracted/starfield/geometries"),
            material_path_rewriter: None,
            target_skeleton: Some(
                repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
            ),
            translation_maps_dir: Some(
                repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            ),
            weapon_role: None,
            cloth_companions: Vec::new(),
        };
        let report = crate::sf_convert::convert_starfield_nif(&source, &output, &options).unwrap();
        assert_eq!(report.cloth_blobs, 1);
        assert_eq!(report.custom_cloth_bones, 1);
        assert_eq!(report.collision_fallback, 0);
        let converted = NifFile::load(output).unwrap();
        let cloth = &converted.blocks[converted.find_blocks("BSClothExtraData")[0]];
        let blob =
            crate::cloth::byte_array_to_bytes(cloth.get_field("Binary Data").unwrap()).unwrap();
        let hkx = havok_native::hkx::HkxFile::read(&blob).unwrap();
        assert_eq!(hkx.contents_version(), "hk_2014.1.0-r1");
        let custom = converted
            .blocks
            .iter()
            .find(|block| {
                block.get_field("Name") == Some(&NifValue::String("SS_L_Cloth_01".into()))
            })
            .expect("authored simulation bone");
        assert!(converted.find_blocks("BSSkin::Instance").iter().any(|id| {
            matches!(converted.blocks[*id].get_field("Bones"), Some(NifValue::Array(bones)) if bones.contains(&NifValue::Ref(custom.block_id as i32)))
        }));
        let binds = crate::skeleton_repose::collect_bind_matrices_by_name(&converted);
        let (count, total) =
            crate::skeleton_repose::skeleton_bind_consistency(&converted, &binds, 0.001);
        assert!(count > 10);
        assert_eq!(count, total);
    }

    #[test]
    fn authored_cloth_bone_is_installed_and_used_by_the_skin() {
        let (mut source, shape, mut mesh, mut dest, mut context) = fixture();
        let names_id = reference(
            match source.blocks[shape].get_field("Extra Data List").unwrap() {
                NifValue::Array(extras) => extras.first(),
                _ => panic!(),
            },
        )
        .unwrap();
        let NifValue::Array(names) = source.blocks[names_id].get_field_mut("Bones").unwrap() else {
            panic!()
        };
        names[5] = NifValue::String("AuthoredClothBone".into());
        mesh.bone_influences.as_mut().unwrap()[0].slots = vec![(5, 1.0)];
        assert!(
            context
                .convert(&source, &source.blocks[shape], &mesh, &mut dest)
                .unwrap_err()
                .contains("unmapped Starfield skin bone")
        );
        let mut local = local_of(&NifBlock::new(0, "NiNode"));
        local[2][3] = 1.0;
        let world = mat4_mul(&context.bones["target0"].2, &local);
        context
            .install_cloth_bones(
                &mut dest,
                vec![crate::sf_cloth::ClothBoneBinding {
                    source_name: "AuthoredClothBone".into(),
                    target_name: "AuthoredClothBone".into(),
                    world,
                    custom: Some(crate::sf_cloth::ClothBoneNode {
                        parent: "Target0".into(),
                        local,
                    }),
                }],
            )
            .unwrap();
        assert_eq!(context.custom_cloth_bones, 1);
        let skin = context
            .convert(&source, &source.blocks[shape], &mesh, &mut dest)
            .unwrap();
        let (name, id, _) = &context.bones["authoredclothbone"];
        assert_eq!(name, "AuthoredClothBone");
        assert_eq!(local_of(&dest.blocks[*id])[2][3], 1.0);
        assert_eq!(
            dest.blocks[skin.instance].get_field("Bones"),
            Some(&NifValue::Array(vec![NifValue::Ref(*id as i32)]))
        );
        assert!((skin.positions[0][2] - (11.0 + 3.0 * HAVOK_SCALE as f32)).abs() < 0.0001);
    }

    #[test]
    fn cloth_binding_cannot_replace_an_existing_bone_rest_pose() {
        let (_, _, _, mut dest, mut context) = fixture();
        let mut world = context.bones["target0"].2;
        world[0][3] += 1.0;
        let error = context
            .install_cloth_bones(
                &mut dest,
                vec![crate::sf_cloth::ClothBoneBinding {
                    source_name: "Source0".into(),
                    target_name: "Target0".into(),
                    world,
                    custom: None,
                }],
            )
            .unwrap_err();
        assert!(error.contains("conflicting rest poses"));
        assert_eq!(context.custom_cloth_bones, 0);
    }

    #[test]
    #[ignore = "requires extracted Starfield and FO4 assets"]
    fn real_constellation_torso_retains_a_bound_skin_after_roundtrip() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = repo.join("extracted/starfield/meshes/Clothes/Spacesuit_Constellation/Spacesuit_Constellation_Torso_M.nif");
        let target =
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif");
        let maps = repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps");
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("torso.nif");
        let options = crate::sf_convert::SfConvertOptions {
            geometries_root: repo.join("extracted/starfield/geometries"),
            material_path_rewriter: None,
            target_skeleton: Some(target),
            translation_maps_dir: Some(maps),
            weapon_role: None,
            cloth_companions: Vec::new(),
        };
        let report = crate::sf_convert::convert_starfield_nif(&source, &output, &options).unwrap();
        assert_eq!(report.skinned_shapes, 1);
        assert_eq!(report.collision_fallback, 0);
        let converted = NifFile::load(output).unwrap();
        assert_eq!(converted.find_blocks("BSSkin::Instance").len(), 1);
        let binds = crate::skeleton_repose::collect_bind_matrices_by_name(&converted);
        let (count, total) =
            crate::skeleton_repose::skeleton_bind_consistency(&converted, &binds, 0.001);
        assert!(count >= 10, "{count} bones");
        assert_eq!(count, total);
        println!(
            "Constellation torso: {} reduced vertices; max discarded {}",
            report.reduced_skin_vertices, report.max_discarded_skin_weight
        );
    }

    #[test]
    #[ignore = "requires extracted Starfield and FO4 assets"]
    fn real_ryujin_sleeves_use_verified_companion_bones_without_duplicate_cloth() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let family =
            repo.join("extracted/starfield/meshes/Clothes/outfit_ryujin_businesssuit_manager_02");
        let source = family.join("Outfit_Ryujin_BusinessSuit_Manager_UpperBody_02_F_1st.nif");
        let owner = family.join("Outfit_Ryujin_BusinessSuit_Manager_Sleeves_02_F_1st.nif");
        let dir = tempfile::tempdir().unwrap();
        let options = crate::sf_convert::SfConvertOptions {
            geometries_root: repo.join("extracted/starfield/geometries"),
            material_path_rewriter: None,
            target_skeleton: Some(repo.join(
                "extracted/fo4/meshes/actors/character/_1stperson/characterassets/skeleton.nif",
            )),
            translation_maps_dir: Some(
                repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            ),
            weapon_role: None,
            cloth_companions: vec![owner],
        };
        let output = dir.path().join("torso_1st.nif");
        let report = crate::sf_convert::convert_starfield_nif(&source, &output, &options).unwrap();
        assert_eq!(report.skinned_shapes, 2);
        assert_eq!(report.cloth_blobs, 0);
        assert!(report.custom_cloth_bones >= 2);
        let converted = NifFile::load(output).unwrap();
        assert_eq!(converted.find_blocks("BSSkin::Instance").len(), 2);
        assert!(converted.find_blocks("BSClothExtraData").is_empty());
        for name in ["L_Cloth_Sleeve", "R_Cloth_Sleeve"] {
            let bone = converted
                .blocks
                .iter()
                .find(|block| block.get_field("Name") == Some(&NifValue::String(name.into())))
                .expect("companion simulation bone");
            assert!(converted.find_blocks("BSSkin::Instance").iter().any(|id| {
                matches!(converted.blocks[*id].get_field("Bones"), Some(NifValue::Array(bones)) if bones.contains(&NifValue::Ref(bone.block_id as i32)))
            }));
        }
        let binds = crate::skeleton_repose::collect_bind_matrices_by_name(&converted);
        let (count, total) =
            crate::skeleton_repose::skeleton_bind_consistency(&converted, &binds, 0.001);
        assert!(count > 10);
        assert_eq!(count, total);

        let mut changed = NifFile::load(source).unwrap();
        let (data_id, palette_index) = changed
            .blocks
            .iter()
            .find_map(|shape| {
                let instance =
                    reference(shape.get_field("Skin")).and_then(|id| changed.get_block(id))?;
                let names = source_bone_names(&changed, shape, instance).unwrap();
                let index = names.iter().position(|name| name == "L_Cloth_Sleeve")?;
                Some((reference(instance.get_field("Data")).unwrap(), index))
            })
            .unwrap();
        let NifValue::Array(binds) = changed.blocks[data_id].get_field_mut("Bone List").unwrap()
        else {
            panic!()
        };
        let NifValue::Struct(fields) = &mut binds[palette_index] else {
            panic!()
        };
        let mut bind = NifBlock::new(0, "NiNode");
        bind.fields = fields.clone();
        let mut local = local_of(&bind);
        local[0][3] += 0.01;
        set_local(&mut bind, &local);
        *fields = bind.fields;
        let changed_path = dir.path().join("changed.nif");
        changed.save(Some(changed_path.clone())).unwrap();
        let rejected = dir.path().join("rejected.nif");
        let error = crate::sf_convert::convert_starfield_nif(&changed_path, &rejected, &options)
            .unwrap_err();
        assert!(
            error.contains("cloth companion inverse bind disagrees for L_Cloth_Sleeve"),
            "{error}"
        );
        assert!(!rejected.exists());
    }

    #[test]
    #[ignore = "requires extracted Starfield and FO4 assets"]
    fn real_first_person_constellation_torso_uses_first_person_rest_pose() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = repo.join("extracted/starfield/meshes/Clothes/Spacesuit_Constellation/Spacesuit_Constellation_Torso_M_1st.nif");
        let dir = tempfile::tempdir().unwrap();
        let options = crate::sf_convert::SfConvertOptions {
            geometries_root: repo.join("extracted/starfield/geometries"),
            material_path_rewriter: None,
            target_skeleton: Some(repo.join(
                "extracted/fo4/meshes/actors/character/_1stperson/characterassets/skeleton.nif",
            )),
            translation_maps_dir: Some(
                repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            ),
            weapon_role: None,
            cloth_companions: Vec::new(),
        };
        let output = dir.path().join("torso_1st.nif");
        let report = crate::sf_convert::convert_starfield_nif(&source, &output, &options).unwrap();
        assert_eq!(report.skinned_shapes, 1);
        let converted = NifFile::load(output).unwrap();
        let binds = crate::skeleton_repose::collect_bind_matrices_by_name(&converted);
        let (count, total) =
            crate::skeleton_repose::skeleton_bind_consistency(&converted, &binds, 0.001);
        assert!(count >= 10);
        assert_eq!(count, total);
        println!(
            "1P torso: {} reduced vertices; max discarded {}",
            report.reduced_skin_vertices, report.max_discarded_skin_weight
        );
    }
}
