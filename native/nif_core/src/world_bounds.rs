use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::model::{NifBlock, NifFile, NifValue};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorldBoundsError {
    #[error("failed to load NIF {path}: {reason}")]
    Load { path: PathBuf, reason: String },
    #[error("render scene contains no supported geometry")]
    EmptyGeometry,
    #[error("named-node scene requires exactly one root, found {count}")]
    RootCount { count: usize },
    #[error("scene root {block_id} is not a node ({type_name})")]
    InvalidRoot { block_id: usize, type_name: String },
    #[error("block {parent_id} references missing child {child_id}")]
    MissingChild { parent_id: usize, child_id: usize },
    #[error("scene graph contains a cycle through block {block_id}")]
    Cycle { block_id: usize },
    #[error("node {block_id} has no nonempty name")]
    EmptyNodeName { block_id: usize },
    #[error("block {block_id} has unsupported {field} transform data")]
    UnsupportedTransform {
        block_id: usize,
        field: &'static str,
    },
    #[error("block {block_id} has non-finite {field} transform data")]
    NonFiniteTransform {
        block_id: usize,
        field: &'static str,
    },
    #[error("unsupported render geometry block {block_id} ({type_name}): {reason}")]
    UnsupportedGeometry {
        block_id: usize,
        type_name: String,
        reason: String,
    },
    #[error("geometry block {block_id} vertex {vertex_index} is non-finite")]
    NonFiniteVertex {
        block_id: usize,
        vertex_index: usize,
    },
    #[error("aggregate world bounds are non-finite")]
    NonFiniteBounds,
}

pub fn aggregate_render_world_bounds(
    path: impl AsRef<Path>,
) -> Result<WorldBounds, WorldBoundsError> {
    let nif = load_nif(path.as_ref())?;
    let roots = scene_roots(&nif);
    if roots.is_empty() {
        return Err(WorldBoundsError::EmptyGeometry);
    }

    let mut bounds = BoundsAccumulator::default();
    for root in roots {
        collect_render_bounds(
            &nif,
            root,
            Transform::identity(),
            &mut HashSet::new(),
            &mut bounds,
        )?;
    }
    bounds.finish().ok_or(WorldBoundsError::EmptyGeometry)?
}

pub fn aggregate_named_node_world_bounds(
    path: impl AsRef<Path>,
) -> Result<WorldBounds, WorldBoundsError> {
    let nif = load_nif(path.as_ref())?;
    let roots = scene_roots(&nif);
    if roots.len() != 1 {
        return Err(WorldBoundsError::RootCount { count: roots.len() });
    }
    let root = nif
        .get_block(roots[0])
        .ok_or(WorldBoundsError::MissingChild {
            parent_id: roots[0],
            child_id: roots[0],
        })?;
    if !is_node(root) {
        return Err(WorldBoundsError::InvalidRoot {
            block_id: root.block_id,
            type_name: root.type_name.clone(),
        });
    }

    let mut bounds = BoundsAccumulator::default();
    collect_named_node_bounds(
        &nif,
        roots[0],
        Transform::identity(),
        &mut HashSet::new(),
        &mut bounds,
    )?;
    bounds
        .finish()
        .ok_or(WorldBoundsError::RootCount { count: 0 })?
}

fn load_nif(path: &Path) -> Result<NifFile, WorldBoundsError> {
    NifFile::load(path.to_path_buf()).map_err(|error| WorldBoundsError::Load {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

fn scene_roots(nif: &NifFile) -> Vec<usize> {
    nif.header
        .footer_roots
        .iter()
        .filter_map(|root| usize::try_from(*root).ok())
        .collect()
}

fn collect_render_bounds(
    nif: &NifFile,
    block_id: usize,
    parent_world: Transform,
    active: &mut HashSet<usize>,
    bounds: &mut BoundsAccumulator,
) -> Result<(), WorldBoundsError> {
    if !active.insert(block_id) {
        return Err(WorldBoundsError::Cycle { block_id });
    }
    let result = (|| {
        let block = nif
            .get_block(block_id)
            .ok_or(WorldBoundsError::MissingChild {
                parent_id: block_id,
                child_id: block_id,
            })?;
        let world = parent_world.combine(read_transform(block)?);
        validate_transform(block.block_id, world)?;

        if is_supported_geometry(block) {
            let vertices = geometry_vertices(nif, block)?;
            for (vertex_index, vertex) in vertices.iter().copied().enumerate() {
                let world_vertex = world.apply(vertex);
                if !world_vertex.iter().all(|value| value.is_finite()) {
                    return Err(WorldBoundsError::NonFiniteVertex {
                        block_id: block.block_id,
                        vertex_index,
                    });
                }
                bounds.include(world_vertex);
            }
            return Ok(());
        }
        if is_render_geometry(block) {
            return Err(WorldBoundsError::UnsupportedGeometry {
                block_id: block.block_id,
                type_name: block.type_name.clone(),
                reason: "geometry type has no supported vertex decoder".to_string(),
            });
        }
        if !is_node(block) {
            return Ok(());
        }
        for child_id in child_refs(block)? {
            if nif.get_block(child_id).is_none() {
                return Err(WorldBoundsError::MissingChild {
                    parent_id: block.block_id,
                    child_id,
                });
            }
            collect_render_bounds(nif, child_id, world, active, bounds)?;
        }
        Ok(())
    })();
    active.remove(&block_id);
    result
}

fn collect_named_node_bounds(
    nif: &NifFile,
    block_id: usize,
    parent_world: Transform,
    active: &mut HashSet<usize>,
    bounds: &mut BoundsAccumulator,
) -> Result<(), WorldBoundsError> {
    if !active.insert(block_id) {
        return Err(WorldBoundsError::Cycle { block_id });
    }
    let result = (|| {
        let block = nif
            .get_block(block_id)
            .ok_or(WorldBoundsError::MissingChild {
                parent_id: block_id,
                child_id: block_id,
            })?;
        if !is_node(block) {
            return Ok(());
        }
        let name = match block.get_field("Name") {
            Some(NifValue::String(name)) => name.trim_end_matches('\0').trim(),
            _ => "",
        };
        if name.is_empty() {
            return Err(WorldBoundsError::EmptyNodeName {
                block_id: block.block_id,
            });
        }
        let world = parent_world.combine(read_transform(block)?);
        validate_transform(block.block_id, world)?;
        bounds.include(world.apply([0.0; 3]));

        for child_id in child_refs(block)? {
            let child = nif
                .get_block(child_id)
                .ok_or(WorldBoundsError::MissingChild {
                    parent_id: block.block_id,
                    child_id,
                })?;
            if is_node(child) {
                collect_named_node_bounds(nif, child_id, world, active, bounds)?;
            }
        }
        Ok(())
    })();
    active.remove(&block_id);
    result
}

fn child_refs(block: &NifBlock) -> Result<Vec<usize>, WorldBoundsError> {
    match block.get_field("Children") {
        None => Ok(Vec::new()),
        Some(NifValue::Array(children)) => Ok(children
            .iter()
            .filter_map(|child| match child {
                NifValue::Ref(reference) => usize::try_from(*reference).ok(),
                _ => None,
            })
            .collect()),
        Some(_) => Err(WorldBoundsError::UnsupportedGeometry {
            block_id: block.block_id,
            type_name: block.type_name.clone(),
            reason: "Children is not a reference array".to_string(),
        }),
    }
}

fn geometry_vertices(nif: &NifFile, shape: &NifBlock) -> Result<Vec<[f32; 3]>, WorldBoundsError> {
    if shape.type_name == "BSDynamicTriShape" {
        let vertices = vertex_array(shape, "Vertices", None)?;
        if !vertices.is_empty() {
            return Ok(vertices);
        }
    }

    let vertices = vertex_array(shape, "Vertex Data", Some("Vertex"))?;
    if !vertices.is_empty() {
        return Ok(vertices);
    }

    if let Some(data_id) = positive_ref(shape, &["Data"]) {
        let data = nif
            .get_block(data_id)
            .ok_or(WorldBoundsError::MissingChild {
                parent_id: shape.block_id,
                child_id: data_id,
            })?;
        if !matches!(
            data.type_name.as_str(),
            "NiTriShapeData" | "NiTriStripsData"
        ) {
            return Err(unsupported_stream(
                shape,
                "Data does not reference legacy triangle data",
            ));
        }
        let vertices = vertex_array(data, "Vertices", None)?;
        if !vertices.is_empty() {
            return Ok(vertices);
        }
    }

    if let Some(instance_id) = positive_ref(shape, &["Skin Instance", "Skin"]) {
        let instance = nif
            .get_block(instance_id)
            .ok_or(WorldBoundsError::MissingChild {
                parent_id: shape.block_id,
                child_id: instance_id,
            })?;
        if let Some(partition_id) = positive_ref(instance, &["Skin Partition"]) {
            let partition = nif
                .get_block(partition_id)
                .ok_or(WorldBoundsError::MissingChild {
                    parent_id: instance.block_id,
                    child_id: partition_id,
                })?;
            let vertices = vertex_array(partition, "Vertex Data", Some("Vertex"))?;
            if !vertices.is_empty() {
                return Ok(vertices);
            }
        }
    }

    Err(unsupported_stream(
        shape,
        "no supported vertex stream is present",
    ))
}

fn vertex_array(
    block: &NifBlock,
    field: &'static str,
    member: Option<&'static str>,
) -> Result<Vec<[f32; 3]>, WorldBoundsError> {
    let Some(value) = block.get_field(field) else {
        return Ok(Vec::new());
    };
    let NifValue::Array(values) = value else {
        return Err(unsupported_stream(block, "vertex stream is not an array"));
    };
    let mut vertices = Vec::with_capacity(values.len());
    for (vertex_index, value) in values.iter().enumerate() {
        let value = match member {
            Some(member) => match value {
                NifValue::Struct(fields) => fields.get(member).ok_or_else(|| {
                    unsupported_stream(block, "vertex entry has no position member")
                })?,
                _ => {
                    return Err(unsupported_stream(block, "vertex entry is not structured"));
                }
            },
            None => value,
        };
        let vertex = vec3(value)
            .ok_or_else(|| unsupported_stream(block, "vertex position is not a vector"))?;
        if !vertex.iter().all(|component| component.is_finite()) {
            return Err(WorldBoundsError::NonFiniteVertex {
                block_id: block.block_id,
                vertex_index,
            });
        }
        vertices.push(vertex);
    }
    Ok(vertices)
}

fn unsupported_stream(block: &NifBlock, reason: &str) -> WorldBoundsError {
    WorldBoundsError::UnsupportedGeometry {
        block_id: block.block_id,
        type_name: block.type_name.clone(),
        reason: reason.to_string(),
    }
}

fn positive_ref(block: &NifBlock, names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| match block.get_field(name) {
        Some(NifValue::Ref(reference)) => usize::try_from(*reference).ok(),
        _ => None,
    })
}

fn is_node(block: &NifBlock) -> bool {
    crate::schema::SCHEMA.is_subtype_of(&block.type_name, "NiNode")
}

fn is_supported_geometry(block: &NifBlock) -> bool {
    matches!(
        block.type_name.as_str(),
        "BSTriShape" | "BSSubIndexTriShape" | "BSDynamicTriShape" | "NiTriShape" | "NiTriStrips"
    )
}

fn is_render_geometry(block: &NifBlock) -> bool {
    crate::schema::SCHEMA.is_subtype_of(&block.type_name, "NiGeometry")
        || crate::schema::SCHEMA.is_subtype_of(&block.type_name, "BSTriShape")
        || block.type_name == "BSGeometry"
}

#[derive(Clone, Copy)]
struct Transform {
    translation: [f32; 3],
    rotation: [[f32; 3]; 3],
    scale: f32,
}

impl Transform {
    fn identity() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            scale: 1.0,
        }
    }

    fn apply(self, point: [f32; 3]) -> [f32; 3] {
        let rotated = [
            point[0] * self.rotation[0][0]
                + point[1] * self.rotation[1][0]
                + point[2] * self.rotation[2][0],
            point[0] * self.rotation[0][1]
                + point[1] * self.rotation[1][1]
                + point[2] * self.rotation[2][1],
            point[0] * self.rotation[0][2]
                + point[1] * self.rotation[1][2]
                + point[2] * self.rotation[2][2],
        ];
        [
            rotated[0] * self.scale + self.translation[0],
            rotated[1] * self.scale + self.translation[1],
            rotated[2] * self.scale + self.translation[2],
        ]
    }

    fn combine(self, local: Self) -> Self {
        let mut rotation = [[0.0; 3]; 3];
        for (row, output_row) in rotation.iter_mut().enumerate() {
            for (column, output) in output_row.iter_mut().enumerate() {
                *output = local.rotation[row][0] * self.rotation[0][column]
                    + local.rotation[row][1] * self.rotation[1][column]
                    + local.rotation[row][2] * self.rotation[2][column];
            }
        }
        Self {
            translation: self.apply(local.translation),
            rotation,
            scale: self.scale * local.scale,
        }
    }
}

fn read_transform(block: &NifBlock) -> Result<Transform, WorldBoundsError> {
    let translation = match block.get_field("Translation") {
        None => [0.0; 3],
        Some(value) => vec3(value).ok_or(WorldBoundsError::UnsupportedTransform {
            block_id: block.block_id,
            field: "Translation",
        })?,
    };
    validate_finite(block.block_id, "Translation", translation.iter().copied())?;

    let rotation = match block.get_field("Rotation") {
        None => Transform::identity().rotation,
        Some(NifValue::Matrix33(rotation)) => *rotation,
        Some(NifValue::Struct(fields)) => [
            [
                number(fields.get("m11")),
                number(fields.get("m21")),
                number(fields.get("m31")),
            ],
            [
                number(fields.get("m12")),
                number(fields.get("m22")),
                number(fields.get("m32")),
            ],
            [
                number(fields.get("m13")),
                number(fields.get("m23")),
                number(fields.get("m33")),
            ],
        ]
        .map(|row| row.map(|value| value.unwrap_or(f32::NAN))),
        Some(_) => {
            return Err(WorldBoundsError::UnsupportedTransform {
                block_id: block.block_id,
                field: "Rotation",
            });
        }
    };
    validate_finite(
        block.block_id,
        "Rotation",
        rotation.iter().flatten().copied(),
    )?;

    let scale = match block.get_field("Scale") {
        None => 1.0,
        Some(value) => number(Some(value)).ok_or(WorldBoundsError::UnsupportedTransform {
            block_id: block.block_id,
            field: "Scale",
        })?,
    };
    validate_finite(block.block_id, "Scale", [scale])?;
    Ok(Transform {
        translation,
        rotation,
        scale,
    })
}

fn validate_transform(block_id: usize, transform: Transform) -> Result<(), WorldBoundsError> {
    validate_finite(
        block_id,
        "world",
        transform
            .translation
            .into_iter()
            .chain(transform.rotation.into_iter().flatten())
            .chain([transform.scale]),
    )
}

fn validate_finite(
    block_id: usize,
    field: &'static str,
    values: impl IntoIterator<Item = f32>,
) -> Result<(), WorldBoundsError> {
    if values.into_iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(WorldBoundsError::NonFiniteTransform { block_id, field })
    }
}

fn vec3(value: &NifValue) -> Option<[f32; 3]> {
    match value {
        NifValue::Vec3(vector) => Some(*vector),
        NifValue::Vec4(vector) => Some([vector[0], vector[1], vector[2]]),
        NifValue::Struct(fields) => Some([
            number(fields.get("x"))?,
            number(fields.get("y"))?,
            number(fields.get("z"))?,
        ]),
        _ => None,
    }
}

fn number(value: Option<&NifValue>) -> Option<f32> {
    match value? {
        NifValue::Float(value) => Some(*value as f32),
        NifValue::Int(value) => Some(*value as f32),
        NifValue::UInt(value) => Some(*value as f32),
        _ => None,
    }
}

struct BoundsAccumulator {
    min: [f32; 3],
    max: [f32; 3],
    samples: usize,
}

impl Default for BoundsAccumulator {
    fn default() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
            samples: 0,
        }
    }
}

impl BoundsAccumulator {
    fn include(&mut self, point: [f32; 3]) {
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(point[axis]);
            self.max[axis] = self.max[axis].max(point[axis]);
        }
        self.samples += 1;
    }

    fn finish(self) -> Option<Result<WorldBounds, WorldBoundsError>> {
        if self.samples == 0 {
            return None;
        }
        if self
            .min
            .iter()
            .chain(&self.max)
            .any(|value| !value.is_finite())
            || self.min.iter().zip(&self.max).any(|(min, max)| min > max)
        {
            return Some(Err(WorldBoundsError::NonFiniteBounds));
        }
        Some(Ok(WorldBounds {
            min: self.min,
            max: self.max,
        }))
    }
}
