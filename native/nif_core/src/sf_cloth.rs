use crate::model::{NifFile, NifValue};
use crate::skeleton_repose::{
    local_of, mat4_affine_inverse, mat4_mul, node_order_and_parents, BindMatrix,
};
use crate::skin::bone_remap::SkeletonMap;
use havok_native::convert::fo76::{
    migrate_2015_packfile_to_2014_with_warnings, Fo76MigrationOptions,
};
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{self, descriptors::DescriptorRegistry, HkxFile, HkxMember, HkxObject};
use std::collections::{BTreeSet, HashMap};

const UNIT_SCALE: f64 = 69.99125;

pub(crate) struct ClothBoneNode {
    pub parent: String,
    pub local: BindMatrix,
}

pub(crate) struct ClothBoneBinding {
    pub source_name: String,
    pub target_name: String,
    pub world: BindMatrix,
    pub custom: Option<ClothBoneNode>,
}

pub(crate) struct AdaptedCloth {
    pub blob: Vec<u8>,
    pub bones: Vec<ClothBoneBinding>,
    pub unbound_unused: Vec<String>,
    pub runtime_requirements: Vec<&'static str>,
}

#[derive(Default)]
pub(crate) struct ClothRuntimeCapabilities {
    pub bend_mx_v1: bool,
}

const BEND_MX_CAPABILITY: &str = "B21_ConversionRuntime.starfield_bend_mx.v1";
const BEND_MX_MARKER: &str = "B21_SF_BENDMX:1:4:";

fn field<'a>(members: &'a [HkxMember], name: &str) -> Result<&'a HkxValue, String> {
    members
        .iter()
        .find(|m| m.name == name)
        .map(|m| &m.value)
        .ok_or_else(|| format!("Starfield cloth missing {name}"))
}

fn field_mut<'a>(members: &'a mut [HkxMember], name: &str) -> Result<&'a mut HkxValue, String> {
    members
        .iter_mut()
        .find(|m| m.name == name)
        .map(|m| &mut m.value)
        .ok_or_else(|| format!("Starfield cloth missing {name}"))
}

fn array(value: &HkxValue) -> Result<&[HkxValue], String> {
    match value {
        HkxValue::Array(a) => Ok(a),
        _ => Err("Starfield cloth expected array".into()),
    }
}

fn members(value: &HkxValue) -> Result<&[HkxMember], String> {
    value
        .as_object_members()
        .ok_or("Starfield cloth expected struct".into())
}

fn integer(value: &HkxValue) -> Result<i64, String> {
    match value {
        HkxValue::I8(v) => Ok(*v as i64),
        HkxValue::U8(v) => Ok(*v as i64),
        HkxValue::I16(v) => Ok(*v as i64),
        HkxValue::U16(v) => Ok(*v as i64),
        HkxValue::I32(v) => Ok(*v as i64),
        HkxValue::U32(v) => Ok(*v as i64),
        HkxValue::I64(v) => Ok(*v),
        HkxValue::U64(v) => {
            i64::try_from(*v).map_err(|_| "Starfield cloth integer is out of range".into())
        }
        _ => Err("Starfield cloth expected integer".into()),
    }
}

fn index(value: &HkxValue) -> Result<usize, String> {
    usize::try_from(integer(value)?).map_err(|_| "Starfield cloth negative index".into())
}

fn floats(value: &HkxValue) -> Result<Vec<f64>, String> {
    match value {
        HkxValue::F32List(values) if values.iter().all(|v| v.is_finite()) => {
            Ok(values.iter().map(|v| *v as f64).collect())
        }
        _ => Err("Starfield cloth expected finite float vector".into()),
    }
}

fn unique(file: &HkxFile, class: &str) -> Result<usize, String> {
    let ids: Vec<_> = file
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| o.class_name == class)
        .map(|(i, _)| i)
        .collect();
    if ids.len() == 1 {
        Ok(ids[0])
    } else {
        Err(format!(
            "Starfield cloth requires one {class}, found {}",
            ids.len()
        ))
    }
}

fn identity() -> BindMatrix {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]
}

fn qs_matrix(value: &HkxValue, scale: f64) -> Result<BindMatrix, String> {
    let v = floats(value)?;
    if v.len() != 12 {
        return Err("Starfield cloth invalid reference pose".into());
    }
    let (x, y, z, w) = (v[4], v[5], v[6], v[7]);
    let (sx, sy, sz) = (v[8], v[9], v[10]);
    if (x * x + y * y + z * z + w * w - 1.).abs() > 1e-3
        || [sx, sy, sz].iter().any(|s| (*s - 1.).abs() > 1e-3)
    {
        return Err("Starfield cloth nonrigid reference pose is unsupported".into());
    }
    Ok([
        [
            (1. - 2. * (y * y + z * z)) * sx,
            2. * (x * y - z * w) * sy,
            2. * (x * z + y * w) * sz,
            v[0] * scale,
        ],
        [
            2. * (x * y + z * w) * sx,
            (1. - 2. * (x * x + z * z)) * sy,
            2. * (y * z - x * w) * sz,
            v[1] * scale,
        ],
        [
            2. * (x * z - y * w) * sx,
            2. * (y * z + x * w) * sy,
            (1. - 2. * (x * x + y * y)) * sz,
            v[2] * scale,
        ],
        [0., 0., 0., 1.],
    ])
}

fn matrix_qs(m: &BindMatrix) -> Result<HkxValue, String> {
    let mut r = *m;
    let mut scales = [0.; 3];
    for c in 0..3 {
        scales[c] = (0..3).map(|row| m[row][c] * m[row][c]).sum::<f64>().sqrt();
        if scales[c] < 1e-8 {
            return Err("Starfield cloth singular target bone".into());
        }
        for row in 0..3 {
            r[row][c] /= scales[c];
        }
    }
    for a in 0..3 {
        for b in a + 1..3 {
            if (0..3).map(|row| r[row][a] * r[row][b]).sum::<f64>().abs() > 1e-4 {
                return Err("Starfield cloth sheared target bone is unsupported".into());
            }
        }
    }
    let trace = r[0][0] + r[1][1] + r[2][2];
    let q = if trace > 0. {
        let s = (trace + 1.).sqrt() * 2.;
        [
            (r[2][1] - r[1][2]) / s,
            (r[0][2] - r[2][0]) / s,
            (r[1][0] - r[0][1]) / s,
            s / 4.,
        ]
    } else {
        let i = if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
            0
        } else if r[1][1] > r[2][2] {
            1
        } else {
            2
        };
        let j = (i + 1) % 3;
        let k = (i + 2) % 3;
        let s = (1. + r[i][i] - r[j][j] - r[k][k]).sqrt() * 2.;
        let mut q = [0.; 4];
        q[i] = s / 4.;
        q[j] = (r[j][i] + r[i][j]) / s;
        q[k] = (r[k][i] + r[i][k]) / s;
        q[3] = (r[k][j] - r[j][k]) / s;
        q
    };
    let out = [
        m[0][3], m[1][3], m[2][3], 0., q[0], q[1], q[2], q[3], scales[0], scales[1], scales[2], 0.,
    ];
    if !out.iter().all(|v| v.is_finite()) {
        return Err("Starfield cloth nonfinite target pose".into());
    }
    Ok(HkxValue::F32List(out.iter().map(|v| *v as f32).collect()))
}

fn target_worlds(skeleton: &NifFile) -> Result<HashMap<String, (String, BindMatrix)>, String> {
    let (order, parents) = node_order_and_parents(skeleton);
    let mut worlds = HashMap::new();
    let mut named = HashMap::new();
    for id in order {
        let block = &skeleton.blocks[id];
        let mut local = local_of(block);
        let scale = match block.get_field("Scale") {
            Some(NifValue::Float(v)) => *v,
            _ => 1.,
        };
        for row in &mut local[..3] {
            for value in &mut row[..3] {
                *value *= scale;
            }
        }
        let world = mat4_mul(
            &parents
                .get(&id)
                .and_then(|p| worlds.get(p))
                .copied()
                .unwrap_or_else(identity),
            &local,
        );
        worlds.insert(id, world);
        if let Some(NifValue::String(name)) = block.get_field("Name") {
            let name = name.trim_end_matches('\0').to_string();
            if named
                .insert(name.to_ascii_lowercase(), (name, world))
                .is_some()
            {
                return Err("Starfield cloth target has duplicate bone names".into());
            }
        }
    }
    Ok(named)
}

fn triangle_frame(
    points: &[[f64; 4]],
    triangles: &[usize],
    offset: usize,
) -> Result<BindMatrix, String> {
    if offset % 6 != 0 {
        return Err("Starfield cloth unaligned triangle offset".into());
    }
    let t = triangles
        .get(offset / 2..offset / 2 + 3)
        .ok_or("Starfield cloth triangle offset out of range")?;
    let p: Vec<_> = t
        .iter()
        .map(|i| {
            points
                .get(*i)
                .copied()
                .ok_or("Starfield cloth triangle vertex out of range")
        })
        .collect::<Result<_, _>>()?;
    let c: Vec<_> = (0..3).map(|j| (p[0][j] + p[1][j] + p[2][j]) / 3.).collect();
    let a: Vec<_> = (0..3).map(|j| p[0][j] - c[j]).collect();
    let b: Vec<_> = (0..3).map(|j| p[1][j] - c[j]).collect();
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let f = [
        [a[0], b[0], n[0], c[0]],
        [a[1], b[1], n[1], c[1]],
        [a[2], b[2], n[2], c[2]],
        [0., 0., 0., 1.],
    ];
    if mat4_affine_inverse(&f).is_none() {
        return Err("Starfield cloth degenerate deformation triangle".into());
    }
    Ok(f)
}

fn matrix_value(m: &BindMatrix) -> HkxValue {
    HkxValue::F32List(
        (0..4)
            .flat_map(|c| (0..4).map(move |r| m[r][c] as f32))
            .collect(),
    )
}

fn value_matrix(v: &HkxValue) -> Result<BindMatrix, String> {
    let v = floats(v)?;
    if v.len() != 16 {
        return Err("Starfield cloth invalid deformation matrix".into());
    }
    Ok(std::array::from_fn(|r| {
        std::array::from_fn(|c| v[c * 4 + r])
    }))
}

fn residual(a: &BindMatrix, b: &BindMatrix) -> f64 {
    (0..4)
        .flat_map(|r| (0..4).map(move |c| (a[r][c] - b[r][c]).abs()))
        .fold(0., f64::max)
}

fn check_classes(file: &HkxFile) -> Result<(), String> {
    check_classes_with_capabilities(file, &ClothRuntimeCapabilities::default())
}

fn check_classes_with_capabilities(
    file: &HkxFile,
    capabilities: &ClothRuntimeCapabilities,
) -> Result<(), String> {
    for object in file.objects() {
        if capabilities.bend_mx_v1 && object.class_name == "hclBendStiffnessConstraintSetMx" {
            continue;
        }
        if !matches!(
            object.class_name.as_str(),
            "hkRootLevelContainer"
                | "hkaSkeleton"
                | "hclClothData"
                | "hclSimClothData"
                | "hclScratchBufferDefinition"
                | "hclBufferDefinition"
                | "hclTransformSetDefinition"
                | "hclBoneSpaceSkinPNOperator"
                | "hclObjectSpaceSkinPNOperator"
                | "hclMoveParticlesOperator"
                | "hclSimulateOperator"
                | "hclSimpleMeshBoneDeformOperator"
                | "hclCopyVerticesOperator"
                | "hclGatherAllVerticesOperator"
                | "hclClothState"
                | "hclSimClothPose"
                | "hclStandardLinkConstraintSetMx"
                | "hclStretchLinkConstraintSetMx"
                | "hclLocalRangeConstraintSet"
                | "hclCollidable"
                | "hclCapsuleShape"
                | "hclBonePlanesConstraintSet"
                | "hclStateDependencyGraph"
        ) {
            return Err(format!(
                "unsupported Starfield cloth class {}",
                object.class_name
            ));
        }
    }
    Ok(())
}

fn proxy_positions(
    op: &HkxObject,
    worlds: &[BindMatrix],
    count: usize,
    scale: f64,
) -> Result<Vec<[f64; 4]>, String> {
    if op.class_name == "hclObjectSpaceSkinPNOperator" {
        return object_proxy_positions(op, worlds, count, scale).map(|(points, _)| points);
    }
    if index(field(&op.members, "transformSetIndex")?)? != 0 {
        return Err("unsupported Starfield cloth skin transform set".into());
    }
    let subset = array(field(&op.members, "transformSubset")?)?
        .iter()
        .map(index)
        .collect::<Result<Vec<_>, _>>()?;
    let d = members(field(&op.members, "boneSpaceDeformer")?)?;
    if !array(field(&op.members, "localUnpackedPNs")?)?.is_empty() {
        return Err("unsupported Starfield cloth unpacked skin blocks".into());
    }
    let entry_sets = [
        "fourBlendEntries",
        "threeBlendEntries",
        "twoBlendEntries",
        "oneBlendEntries",
    ]
    .map(|key| array(field(d, key)?))
    .into_iter()
    .collect::<Result<Vec<_>, String>>()?;
    let controls = array(field(d, "controlBytes")?)?;
    let blocks = array(field(&op.members, "localPNs")?)?;
    if controls.len() != entry_sets.iter().map(|e| e.len()).sum::<usize>()
        || blocks.len() != controls.len()
        || blocks.is_empty()
        || index(field(d, "startVertexIndex")?)? != 0
        || index(field(d, "endVertexIndex")?)? + 1 != count
        || field(d, "partialWrite")? != &HkxValue::Bool(false)
    {
        return Err("unsupported Starfield cloth skin block layout".into());
    }
    let mut result: Vec<Option<[f64; 4]>> = vec![None; count];
    let mut cursors = [0usize; 4];
    for (control, block) in controls.iter().zip(blocks) {
        let kind = index(control)?;
        let entries = entry_sets
            .get(kind)
            .ok_or("unsupported Starfield cloth skin control byte")?;
        let entry = entries
            .get(cursors[kind])
            .ok_or("Starfield cloth skin control/entry mismatch")?;
        cursors[kind] += 1;
        let influences = 4 - kind;
        let lanes = 16 / influences;
        let e = members(entry)?;
        let b = members(block)?;
        let vertices = array(field(e, "vertexIndices")?)?;
        let bones = array(field(e, "boneIndices")?)?;
        let positions = array(field(b, "localPosition")?)?;
        if vertices.len() != lanes || bones.len() != lanes * influences || positions.len() != 16 {
            return Err("Starfield cloth invalid bone-space blend block".into());
        }
        for lane in 0..lanes {
            let vertex = index(&vertices[lane])?;
            let mut p = [0.; 4];
            for slot in 0..influences {
                let i = lane * influences + slot;
                let local = floats(&positions[i])?;
                if local.len() != 4 {
                    return Err("Starfield cloth invalid local position".into());
                }
                let bone = index(&bones[i])?;
                let bone = if subset.is_empty() {
                    bone
                } else {
                    *subset
                        .get(bone)
                        .ok_or("Starfield cloth subset index out of range")?
                };
                let world = worlds
                    .get(bone)
                    .ok_or("Starfield cloth skin bone out of range")?;
                for row in 0..4 {
                    p[row] += (0..4)
                        .map(|c| world[row][c] * local[c] * if c < 3 { scale } else { 1. })
                        .sum::<f64>();
                }
            }
            if (p[3] - 1.).abs() > 1e-4 {
                return Err("Starfield cloth invalid proxy weights".into());
            }
            let out = result
                .get_mut(vertex)
                .ok_or("Starfield cloth proxy index out of range")?;
            if let Some(previous) = out {
                if (0..3).any(|j| (previous[j] - p[j]).abs() > 1e-5) {
                    return Err("Starfield cloth inconsistent repeated proxy vertex".into());
                }
            }
            *out = Some(p);
        }
    }
    if cursors
        .iter()
        .zip(&entry_sets)
        .any(|(n, entries)| *n != entries.len())
    {
        return Err("Starfield cloth unused bone-space blend entries".into());
    }
    result
        .into_iter()
        .map(|p| p.ok_or("Starfield cloth proxy vertex has no skin binding".into()))
        .collect()
}

fn packed_vector(value: &HkxValue) -> Result<([f64; 4], f64), String> {
    let values = array(field(members(value)?, "values")?)?;
    if values.len() != 4 {
        return Err("Starfield cloth invalid packed vector".into());
    }
    let components = values.iter().map(integer).collect::<Result<Vec<_>, _>>()?;
    if components
        .iter()
        .any(|v| *v < i16::MIN as i64 || *v > i16::MAX as i64)
    {
        return Err("Starfield cloth invalid packed vector component".into());
    }
    let step = f32::from_bits((components[3] as u16 as u32) << 16) as f64 * 65536.;
    if !step.is_finite() || step < 0. || (step == 0. && components[..3].iter().any(|v| *v != 0)) {
        return Err("Starfield cloth invalid packed vector exponent".into());
    }
    Ok((
        [
            components[0] as f64 * step,
            components[1] as f64 * step,
            components[2] as f64 * step,
            1.,
        ],
        step,
    ))
}

fn skin_operator(file: &HkxFile) -> Result<usize, String> {
    let ids: Vec<_> = file
        .objects()
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            matches!(
                o.class_name.as_str(),
                "hclBoneSpaceSkinPNOperator" | "hclObjectSpaceSkinPNOperator"
            )
        })
        .map(|(i, _)| i)
        .collect();
    if ids.len() != 1 {
        return Err(format!(
            "Starfield cloth requires one PN skin operator, found {}",
            ids.len()
        ));
    }
    Ok(ids[0])
}

fn object_proxy_positions(
    op: &HkxObject,
    worlds: &[BindMatrix],
    count: usize,
    scale: f64,
) -> Result<(Vec<[f64; 4]>, f64), String> {
    if index(field(&op.members, "transformSetIndex")?)? != 0 {
        return Err("unsupported Starfield cloth object skin transform set".into());
    }
    let subset = array(field(&op.members, "transformSubset")?)?
        .iter()
        .map(index)
        .collect::<Result<Vec<_>, _>>()?;
    let binds = array(field(&op.members, "boneFromSkinMeshTransforms")?)?;
    let indices: Vec<_> = if subset.is_empty() {
        (0..worlds.len()).collect()
    } else {
        subset
    };
    if binds.len() != indices.len() {
        return Err("Starfield cloth object skin bind count mismatch".into());
    }
    let matrices = binds
        .iter()
        .zip(indices)
        .map(|(bind, bone)| {
            let mut bind = value_matrix(bind)?;
            for row in 0..3 {
                bind[row][3] *= scale;
            }
            Ok(mat4_mul(
                worlds
                    .get(bone)
                    .ok_or("Starfield cloth object skin bone out of range")?,
                &bind,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let d = members(field(&op.members, "objectSpaceDeformer")?)?;
    for key in [
        "eightBlendEntries",
        "sevenBlendEntries",
        "sixBlendEntries",
        "fiveBlendEntries",
    ] {
        if let Ok(value) = field(d, key) {
            if !array(value)?.is_empty() {
                return Err(format!("unsupported Starfield cloth object deformer {key}"));
            }
        }
    }
    let entry_sets = [
        "fourBlendEntries",
        "threeBlendEntries",
        "twoBlendEntries",
        "oneBlendEntries",
    ]
    .map(|key| array(field(d, key)?))
    .into_iter()
    .collect::<Result<Vec<_>, String>>()?;
    let packed = array(field(&op.members, "localPNs")?)?;
    let unpacked = array(field(&op.members, "localUnpackedPNs")?)?;
    let blocks = if packed.is_empty() { unpacked } else { packed };
    let controls = array(field(d, "controlBytes")?)?;
    if (!packed.is_empty() && !unpacked.is_empty())
        || blocks.len() != controls.len()
        || controls.len() != entry_sets.iter().map(|e| e.len()).sum::<usize>()
        || index(field(d, "startVertexIndex")?)? != 0
        || index(field(d, "endVertexIndex")?)? + 1 != count
        || field(d, "partialWrite")? != &HkxValue::Bool(false)
    {
        return Err("unsupported Starfield cloth object skin block layout".into());
    }
    let mut cursors = [0usize; 4];
    let mut result: Vec<Option<[f64; 4]>> = vec![None; count];
    let mut quantization_bound = 0f64;
    for (control, block) in controls.iter().zip(blocks) {
        let kind = index(control)?;
        let entries = entry_sets
            .get(kind)
            .ok_or("unsupported Starfield cloth object skin control")?;
        let entry = members(
            entries
                .get(cursors[kind])
                .ok_or("Starfield cloth object skin entry mismatch")?,
        )?;
        cursors[kind] += 1;
        let influences = 4 - kind;
        let vertices = array(field(entry, "vertexIndices")?)?;
        let bones = array(field(entry, "boneIndices")?)?;
        let positions = array(field(members(block)?, "localPosition")?)?;
        let weights = if influences == 1 {
            None
        } else {
            Some(array(field(entry, "boneWeights")?)?)
        };
        if vertices.len() != 16
            || positions.len() != 16
            || bones.len() != 16 * influences
            || weights.is_some_and(|w| w.len() != bones.len())
        {
            return Err("Starfield cloth invalid object-space blend block".into());
        }
        for lane in 0..16 {
            let (mut local, step) = if packed.is_empty() {
                let v: [f64; 4] = floats(&positions[lane])?
                    .try_into()
                    .map_err(|_| "Starfield cloth invalid unpacked object position")?;
                (v, 0.)
            } else {
                packed_vector(&positions[lane])?
            };
            for x in &mut local[..3] {
                *x *= scale;
            }
            local[3] = 1.;
            let mut point = [0.; 4];
            let mut sum = 0;
            let mut error = [0.; 3];
            for slot in 0..influences {
                let i = lane * influences + slot;
                let weight = if let Some(w) = weights {
                    index(&w[i])?
                } else {
                    255
                };
                if weight > 255 {
                    return Err("Starfield cloth invalid object skin weight".into());
                }
                sum += weight;
                let matrix = matrices
                    .get(index(&bones[i])?)
                    .ok_or("Starfield cloth object skin matrix out of range")?;
                for row in 0..4 {
                    point[row] += weight as f64 / 255.
                        * (0..4).map(|c| matrix[row][c] * local[c]).sum::<f64>();
                    if row < 3 {
                        error[row] += weight as f64 / 255.
                            * step
                            * scale
                            * matrix[row][..3].iter().map(|v| v.abs()).sum::<f64>();
                    }
                }
            }
            if sum != 255 {
                return Err("Starfield cloth object skin weights do not sum to 255".into());
            }
            quantization_bound = quantization_bound.max(error.into_iter().fold(0., f64::max));
            let out = result
                .get_mut(index(&vertices[lane])?)
                .ok_or("Starfield cloth object proxy vertex out of range")?;
            if out.is_some_and(|old| (0..3).any(|j| (old[j] - point[j]).abs() > 1e-5)) {
                return Err("Starfield cloth inconsistent repeated object proxy vertex".into());
            }
            *out = Some(point);
        }
    }
    if cursors
        .iter()
        .zip(entry_sets)
        .any(|(n, entries)| *n != entries.len())
    {
        return Err("Starfield cloth unused object-space blend entries".into());
    }
    Ok((
        result
            .into_iter()
            .map(|p| p.ok_or("Starfield cloth object proxy vertex has no skin binding".into()))
            .collect::<Result<Vec<_>, String>>()?,
        quantization_bound,
    ))
}

fn skin_vertex_count(file: &HkxFile, op: &HkxObject) -> Result<usize, String> {
    let cloth = &file.objects()[unique(file, "hclClothData")?].members;
    let buffers = array(field(cloth, "bufferDefinitions")?)?;
    let id = index(field(&op.members, "outputBufferIndex")?)?;
    let HkxValue::Pointer(Some(object)) = buffers
        .get(id)
        .ok_or("Starfield cloth skin buffer out of range")?
    else {
        return Err("Starfield cloth invalid skin buffer reference".into());
    };
    index(field(&file.objects()[*object].members, "numVertices")?)
}

fn particle_vertices(file: &HkxFile, count: usize, vertices: usize) -> Result<Vec<usize>, String> {
    let gathers: Vec<_> = file
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclGatherAllVerticesOperator")
        .collect();
    if gathers.is_empty() && count == vertices {
        return Ok((0..count).collect());
    }
    if gathers.len() != 1 {
        return Err("unsupported Starfield cloth particle gather count".into());
    }
    let m = &gathers[0].members;
    if index(field(m, "inputBufferIdx")?)? != 0
        || index(field(m, "outputBufferIdx")?)? != 1
        || field(m, "partialGather")? != &HkxValue::Bool(false)
    {
        return Err("unsupported Starfield cloth particle gather buffer contract".into());
    }
    let map = array(field(m, "vertexInputFromVertexOutput")?)?
        .iter()
        .map(index)
        .collect::<Result<Vec<_>, _>>()?;
    if map.len() != count || map.iter().any(|v| *v >= vertices) {
        return Err("Starfield cloth particle gather index out of range".into());
    }
    Ok(map)
}

fn point_values(value: &HkxValue) -> Result<Vec<[f64; 4]>, String> {
    array(value)?
        .iter()
        .map(|p| {
            let p = floats(p)?;
            p.try_into()
                .map_err(|_| "Starfield cloth invalid proxy pose".into())
        })
        .collect()
}

fn scaled_number(value: &mut HkxValue, scale: f64) -> Result<(), String> {
    let HkxValue::F32(v) = value else {
        return Err("Starfield cloth expected scalar".into());
    };
    if !v.is_finite() {
        return Err("Starfield cloth nonfinite scalar".into());
    }
    *v = (*v as f64 * scale) as f32;
    Ok(())
}

fn scale_fields(m: &mut [HkxMember], names: &[&str]) -> Result<(), String> {
    for member in m {
        if names.contains(&member.name.as_str()) {
            scaled_number(&mut member.value, UNIT_SCALE)?;
        }
    }
    Ok(())
}

fn scale_vector(value: &mut HkxValue, scale: f64) -> Result<(), String> {
    let HkxValue::F32List(v) = value else {
        return Err("Starfield cloth invalid length vector".into());
    };
    if v.len() != 4 || v.iter().any(|x| !x.is_finite()) {
        return Err("Starfield cloth invalid length vector".into());
    }
    for x in &mut v[..3] {
        *x = (*x as f64 * scale) as f32;
    }
    Ok(())
}

fn transform_matrix(value: &HkxValue) -> Result<BindMatrix, String> {
    let mut m = value_matrix(value)?;
    m[3] = [0., 0., 0., 1.];
    matrix_qs(&m)?;
    Ok(m)
}

fn adapt_collidables(
    file: &mut HkxFile,
    source_worlds: &[BindMatrix],
    target_worlds: &[BindMatrix],
    used: &BTreeSet<usize>,
    canonical_corrections: Option<&[BindMatrix]>,
) -> Result<(), String> {
    let sim_id = unique(file, "hclSimClothData")?;
    let sim = &file.objects()[sim_id].members;
    let collidables = array(field(sim, "perInstanceCollidables")?)?.to_vec();
    let mapping = members(field(sim, "collidableTransformMap")?)?;
    let indices = array(field(mapping, "transformIndices")?)?
        .iter()
        .map(index)
        .collect::<Result<Vec<_>, _>>()?;
    let offsets = array(field(mapping, "offsets")?)?.to_vec();
    if indices.len() != collidables.len()
        || offsets.len() != collidables.len()
        || (!indices.is_empty() && index(field(mapping, "transformSetIndex")?)? != 0)
    {
        return Err("unsupported Starfield cloth collidable binding count/set".into());
    }
    let mut new_offsets = Vec::new();
    let mut shape_ids = BTreeSet::new();
    let mut collider_ids = BTreeSet::new();
    for ((collider, bone), offset) in collidables.iter().zip(indices).zip(&offsets) {
        let HkxValue::Pointer(Some(id)) = collider else {
            return Err("Starfield cloth invalid collidable reference".into());
        };
        if !used.contains(&bone) {
            return Err("Starfield cloth collidable bone missing usage binding".into());
        }
        if !collider_ids.insert(*id) {
            return Err("unsupported Starfield cloth repeated collidable binding".into());
        }
        let object = file
            .objects()
            .get(*id)
            .ok_or("Starfield cloth collidable reference out of range")?;
        if object.class_name != "hclCollidable" {
            return Err("Starfield cloth invalid collidable class".into());
        }
        let m = &object.members;
        for name in [
            "pinchDetectionEnabled",
            "virtualCollisionPointCollisionEnabled",
        ] {
            if let Ok(v) = field(m, name) {
                if v != &HkxValue::Bool(false) {
                    return Err(format!("unsupported Starfield cloth collidable {name}"));
                }
            }
        }
        if let Ok(v) = field(m, "enabled") {
            if v != &HkxValue::Bool(true) {
                return Err("unsupported Starfield cloth disabled collidable".into());
            }
        }
        if let Ok(v) = field(m, "userData") {
            if integer(v)? != 0 {
                return Err("unsupported Starfield cloth collidable user data".into());
            }
        }
        for name in ["linearVelocity", "angularVelocity"] {
            if floats(field(m, name)?)?.iter().any(|v| v.abs() > 1e-8) {
                return Err("unsupported Starfield cloth authored collider velocity".into());
            }
        }
        let mut local = transform_matrix(offset)?;
        let source = mat4_mul(
            source_worlds
                .get(bone)
                .ok_or("Starfield cloth collider bone out of range")?,
            &local,
        );
        let actual = transform_matrix(field(m, "transform")?)?;
        if residual(&source, &actual) > 2e-4 {
            return Err("unsupported Starfield cloth authored collider frame".into());
        }
        for row in 0..3 {
            local[row][3] *= UNIT_SCALE;
        }
        if let Some(corrections) = canonical_corrections {
            local = mat4_mul(&corrections[bone], &local);
        }
        let target = mat4_mul(
            target_worlds
                .get(bone)
                .ok_or("Starfield cloth target collider bone out of range")?,
            &local,
        );
        let HkxValue::Pointer(Some(shape)) = field(m, "shape")? else {
            return Err("Starfield cloth invalid collider shape".into());
        };
        shape_ids.insert(*shape);
        let m = &mut file.objects_mut()[*id].members;
        *field_mut(m, "transform")? = matrix_value(&target);
        scale_fields(m, &["pinchDetectionRadius"])?;
        new_offsets.push(matrix_value(&local));
    }
    if file
        .objects()
        .iter()
        .enumerate()
        .any(|(i, o)| o.class_name == "hclCollidable" && !collider_ids.contains(&i))
    {
        return Err("unsupported Starfield cloth unbound collider".into());
    }
    for id in shape_ids {
        let shape = file
            .objects_mut()
            .get_mut(id)
            .ok_or("Starfield cloth collider shape out of range")?;
        if shape.class_name != "hclCapsuleShape" {
            return Err(format!(
                "unsupported Starfield cloth collision shape {}",
                shape.class_name
            ));
        }
        let m = &mut shape.members;
        let start = floats(field(m, "start")?)?;
        let end = floats(field(m, "end")?)?;
        let dir = floats(field(m, "dir")?)?;
        if start.len() != 4 || end.len() != 4 || dir.len() != 4 {
            return Err("Starfield cloth invalid capsule vector".into());
        }
        let length = (0..3)
            .map(|j| (end[j] - start[j]).powi(2))
            .sum::<f64>()
            .sqrt();
        let HkxValue::F32(inverse) = field(m, "capLenSqrdInv")? else {
            return Err("Starfield cloth invalid capsule length cache".into());
        };
        if length < 1e-8
            || (*inverse as f64 * length * length - 1.).abs() > 1e-4
            || (0..3).any(|j| ((end[j] - start[j]) / length - dir[j]).abs() > 1e-4)
        {
            return Err("unsupported Starfield cloth capsule direction/length cache".into());
        }
        for name in ["start", "end"] {
            scale_vector(field_mut(m, name)?, UNIT_SCALE)?;
        }
        scale_fields(m, &["radius"])?;
        scaled_number(field_mut(m, "capLenSqrdInv")?, 1. / UNIT_SCALE.powi(2))?;
    }
    let m = &mut file.objects_mut()[sim_id].members;
    let map = field_mut(m, "collidableTransformMap")?
        .as_object_members_mut()
        .unwrap();
    *field_mut(map, "offsets")? = HkxValue::Array(new_offsets);
    let HkxValue::Array(pinching) = field_mut(m, "collidablePinchingDatas")? else {
        unreachable!()
    };
    for data in pinching {
        let p = data
            .as_object_members_mut()
            .ok_or("Starfield cloth invalid collider pinch data")?;
        if field(p, "pinchDetectionEnabled")? != &HkxValue::Bool(false) {
            return Err("unsupported Starfield cloth collider pinch detection".into());
        }
        scale_fields(p, &["pinchDetectionRadius"])?;
    }
    Ok(())
}

fn rebase_links(
    object: &mut HkxObject,
    source: &[[f64; 4]],
    target: &[[f64; 4]],
) -> Result<(), String> {
    let HkxValue::Array(batches) = field_mut(&mut object.members, "batches")? else {
        unreachable!()
    };
    for batch in batches {
        let m = batch
            .as_object_members_mut()
            .ok_or("Starfield cloth invalid link batch")?;
        let a = array(field(m, "particlesA")?)?
            .iter()
            .map(index)
            .collect::<Result<Vec<_>, _>>()?;
        let b = array(field(m, "particlesB")?)?
            .iter()
            .map(index)
            .collect::<Result<Vec<_>, _>>()?;
        let rest = array(field(m, "restLengths")?)?;
        if a.len() != 4 || b.len() != 4 || rest.len() != 4 {
            return Err("unsupported Starfield cloth source link batch width".into());
        }
        let scales = a
            .iter()
            .zip(&b)
            .map(|(a, b)| link_scale(source, target, *a, *b))
            .collect::<Result<Vec<_>, _>>()?;
        for member in m {
            let HkxValue::Array(values) = &mut member.value else {
                return Err("Starfield cloth invalid link batch member".into());
            };
            if values.len() != 4 {
                return Err("Starfield cloth inconsistent link batch width".into());
            }
            let inert = match member.name.as_str() {
                "restLengths" => {
                    for (value, scale) in values.iter_mut().zip(&scales) {
                        scaled_number(value, *scale)?;
                    }
                    values[0].clone()
                }
                "particlesA" | "particlesB" => values[0].clone(),
                "stiffnesses" | "stiffnessesA" | "stiffnessesB" => HkxValue::F32(0.),
                _ => {
                    return Err(format!(
                        "unsupported Starfield cloth link batch member {}",
                        member.name
                    ));
                }
            };
            // FO4 samples all sixteen distances before applying four SIMD groups.
            // Keep source batch boundaries; additional lanes have zero correction.
            values.resize(16, inert);
        }
    }
    let HkxValue::Array(links) = field_mut(&mut object.members, "singles")? else {
        unreachable!()
    };
    for link in links {
        let m = link
            .as_object_members_mut()
            .ok_or("Starfield cloth invalid link")?;
        let a = index(field(m, "particleA")?)?;
        let b = index(field(m, "particleB")?)?;
        scaled_number(
            field_mut(m, "restLength")?,
            link_scale(source, target, a, b)?,
        )?;
    }
    Ok(())
}

fn link_scale(source: &[[f64; 4]], target: &[[f64; 4]], a: usize, b: usize) -> Result<f64, String> {
    let distance = |p: &[[f64; 4]]| -> Result<f64, String> {
        let a = p
            .get(a)
            .ok_or("Starfield cloth link particle out of range")?;
        let b = p
            .get(b)
            .ok_or("Starfield cloth link particle out of range")?;
        Ok((0..3).map(|j| (a[j] - b[j]).powi(2)).sum::<f64>().sqrt())
    };
    let old = distance(source)?;
    let new = distance(target)?;
    if old < 1e-8 || new < 1e-8 || !old.is_finite() || !new.is_finite() {
        return Err("Starfield cloth zero-length/nonfinite authored link is unsupported".into());
    }
    Ok(new / old)
}

fn usage_indices(file: &HkxFile, count: usize) -> Result<BTreeSet<usize>, String> {
    let mut used = BTreeSet::new();
    for state in file
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclClothState")
    {
        for set in array(field(&state.members, "usedTransformSets")?)? {
            let set = members(set)?;
            if index(field(set, "transformSetIndex")?)? != 0 {
                return Err("unsupported Starfield cloth state transform set".into());
            }
            let usage = members(field(set, "transformSetUsage")?)?;
            for tracker in array(field(usage, "perComponentTransformTrackers")?)? {
                for key in ["read", "readBeforeWrite", "written"] {
                    let storage =
                        members(field(members(field(members(tracker)?, key)?)?, "storage")?)?;
                    if index(field(storage, "numBits")?)? != count {
                        return Err(
                            "Starfield cloth usage bit count disagrees with skeleton".into()
                        );
                    }
                    for (word, value) in array(field(storage, "words")?)?.iter().enumerate() {
                        let bits = integer(value)? as u32;
                        for bit in 0..32 {
                            if bits & (1 << bit) != 0 {
                                let i = word * 32 + bit;
                                if i >= count {
                                    return Err("Starfield cloth usage index out of range".into());
                                }
                                used.insert(i);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(used)
}

fn adapt_known_fields(
    file: &mut HkxFile,
    source: &[[f64; 4]],
    target: &[[f64; 4]],
    skin_vertices: usize,
) -> Result<(), String> {
    let mut links = 0;
    for object in file.objects_mut() {
        let m = &mut object.members;
        match object.class_name.as_str() {
            "hclSimClothData" => {
                for key in ["antiPinchConstraintSets"] {
                    if !array(field(m, key)?)?.is_empty() {
                        return Err(format!("unsupported Starfield cloth {key}"));
                    }
                }
                for key in ["landscapeCollisionEnabled", "pinchDetectionEnabled"] {
                    if field(m, key)? != &HkxValue::Bool(false) {
                        return Err(format!("unsupported Starfield cloth {key}"));
                    }
                }
                if !array(field(
                    members(field(m, "virtualCollisionPointsData")?)?,
                    "blocks",
                )?)?
                .is_empty()
                {
                    return Err("unsupported Starfield cloth virtual collision points".into());
                }
                let info = field_mut(m, "simulationInfo")?
                    .as_object_members_mut()
                    .ok_or("Starfield cloth invalid simulation info")?;
                let HkxValue::F32List(gravity) = field_mut(info, "gravity")? else {
                    return Err("Starfield cloth invalid gravity".into());
                };
                if gravity.len() != 4 || gravity.iter().any(|v| !v.is_finite()) {
                    return Err("Starfield cloth invalid gravity".into());
                }
                for v in &mut gravity[..3] {
                    *v = (*v as f64 * UNIT_SCALE) as f32;
                }
                scale_fields(info, &["collisionTolerance"])?;
                let HkxValue::Array(particles) = field_mut(m, "particleDatas")? else {
                    unreachable!()
                };
                if particles.len() != source.len() {
                    return Err("Starfield cloth particle/proxy counts disagree".into());
                }
                for p in particles {
                    scale_fields(
                        p.as_object_members_mut()
                            .ok_or("Starfield cloth invalid particle")?,
                        &["radius"],
                    )?;
                }
                scale_fields(m, &["maxParticleRadius"])?;
                let transfer = field_mut(m, "transferMotionData")?
                    .as_object_members_mut()
                    .ok_or("Starfield cloth invalid transfer motion")?;
                if index(field(transfer, "transformSetIndex")?)? != 0 {
                    return Err("unsupported Starfield cloth transfer transform set".into());
                }
                scale_fields(transfer, &["minTranslationSpeed", "maxTranslationSpeed"])?;
                let landscape = field_mut(m, "landscapeCollisionData")?
                    .as_object_members_mut()
                    .ok_or("Starfield cloth invalid landscape collision")?;
                scale_fields(
                    landscape,
                    &[
                        "landscapeRadius",
                        "pinchDetectionRadius",
                        "collisionTolerance",
                    ],
                )?;
            }
            "hclStandardLinkConstraintSetMx" | "hclStretchLinkConstraintSetMx" => {
                links +=
                    array(field(m, "singles")?)?.len() + 4 * array(field(m, "batches")?)?.len();
                rebase_links(object, source, target)?;
            }
            "hclBonePlanesConstraintSet" => {
                if index(field(m, "transformSetIndex")?)? != 0 {
                    return Err("unsupported Starfield cloth plane transform set".into());
                }
                let HkxValue::Array(planes) = field_mut(m, "bonePlanes")? else {
                    unreachable!()
                };
                for plane in planes {
                    let p = plane
                        .as_object_members_mut()
                        .ok_or("Starfield cloth invalid bone plane")?;
                    if index(field(p, "particleIndex")?)? >= source.len() {
                        return Err("Starfield cloth plane particle out of range".into());
                    }
                    let HkxValue::F32List(eq) = field_mut(p, "planeEquationBone")? else {
                        return Err("Starfield cloth invalid bone plane equation".into());
                    };
                    if eq.len() != 4
                        || eq.iter().any(|v| !v.is_finite())
                        || (eq[..3].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() - 1.).abs()
                            > 1e-3
                    {
                        return Err("Starfield cloth nonunit bone plane".into());
                    }
                    eq[3] = (eq[3] as f64 * UNIT_SCALE) as f32;
                }
            }
            "hclLocalRangeConstraintSet" => {
                if index(field(m, "referenceMeshBufferIdx")?)? != 0
                    || index(field(m, "shapeType")?)? != 0
                    || !array(field(m, "localStiffnessConstraints")?)?.is_empty()
                {
                    return Err("unsupported Starfield cloth local range contract".into());
                }
                let HkxValue::Array(constraints) = field_mut(m, "localConstraints")? else {
                    unreachable!()
                };
                for constraint in constraints {
                    let c = constraint
                        .as_object_members_mut()
                        .ok_or("Starfield cloth invalid local range constraint")?;
                    if index(field(c, "particleIndex")?)? >= source.len()
                        || index(field(c, "referenceVertex")?)? >= skin_vertices
                    {
                        return Err("Starfield cloth local range index out of range".into());
                    }
                    scale_fields(
                        c,
                        &["maximumDistance", "maxNormalDistance", "minNormalDistance"],
                    )?;
                }
            }
            "hclBoneSpaceSkinPNOperator" => {
                let HkxValue::Array(blocks) = field_mut(m, "localPNs")? else {
                    unreachable!()
                };
                for block in blocks {
                    let b = block.as_object_members_mut().unwrap();
                    let HkxValue::Array(positions) = field_mut(b, "localPosition")? else {
                        unreachable!()
                    };
                    for p in positions {
                        let HkxValue::F32List(p) = p else {
                            unreachable!()
                        };
                        for v in &mut p[..3] {
                            *v = (*v as f64 * UNIT_SCALE) as f32;
                        }
                    }
                }
            }
            "hclObjectSpaceSkinPNOperator" => {
                let packed = array(field(m, "localPNs")?)?.to_vec();
                if packed.is_empty() {
                    return Err("unsupported Starfield cloth source unpacked object skin".into());
                }
                let mut unpacked = Vec::new();
                for block in &packed {
                    let block = members(block)?;
                    let positions = array(field(block, "localPosition")?)?
                        .iter()
                        .map(|p| {
                            let (mut p, _) = packed_vector(p)?;
                            for x in &mut p[..3] {
                                *x *= UNIT_SCALE;
                            }
                            Ok(HkxValue::F32List(p.into_iter().map(|v| v as f32).collect()))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let normals = array(field(block, "localNormal")?)?
                        .iter()
                        .map(|n| {
                            let (mut n, _) = packed_vector(n)?;
                            n[3] = 0.;
                            Ok(HkxValue::F32List(n.into_iter().map(|v| v as f32).collect()))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let mut fields = block.to_vec();
                    *field_mut(&mut fields, "localPosition")? = HkxValue::Array(positions);
                    *field_mut(&mut fields, "localNormal")? = HkxValue::Array(normals);
                    unpacked.push(HkxValue::TypedObject {
                        class_name: "hclObjectSpaceDeformerLocalBlockUnpackedPN".into(),
                        members: fields,
                    });
                }
                *field_mut(m, "localPNs")? = HkxValue::Array(vec![]);
                *field_mut(m, "localUnpackedPNs")? = HkxValue::Array(unpacked);
                let HkxValue::Array(binds) = field_mut(m, "boneFromSkinMeshTransforms")? else {
                    unreachable!()
                };
                for bind in binds {
                    let mut matrix = value_matrix(bind)?;
                    for row in 0..3 {
                        matrix[row][3] *= UNIT_SCALE;
                    }
                    *bind = matrix_value(&matrix);
                }
            }
            "hclClothData" => {
                for key in ["actions", "stateTransitions"] {
                    if !array(field(m, key)?)?.is_empty() {
                        return Err(format!("unsupported Starfield cloth {key}"));
                    }
                }
            }
            "hclSimulateOperator" => {
                let configs = array(field(m, "simulateOpConfigs")?)?;
                if configs.len() != 1 {
                    return Err("unsupported Starfield cloth solver configurations".into());
                }
                let c = members(&configs[0])?;
                if integer(field(c, "subSteps")?)? <= 0
                    || integer(field(c, "numberOfSolveIterations")?)? <= 0
                {
                    return Err("Starfield cloth invalid solver iterations".into());
                }
            }
            _ => {}
        }
    }
    if links == 0 {
        return Err("unsupported Starfield cloth with no authored structural links".into());
    }
    Ok(())
}

fn validate_adapted(
    file: &HkxFile,
    parents: &[i64],
    points: &[[f64; 4]],
    triangles: &[usize],
    worlds: &[BindMatrix],
    used: &BTreeSet<usize>,
    expected_pairs: &[HkxValue],
    target_names: &[String],
    state_usage: &[Vec<HkxMember>],
    canonical_proxy: Option<&[[f64; 4]]>,
) -> Result<(), String> {
    if file.contents_version() != "hk_2014.1.0-r1" {
        return Err("Starfield cloth target version failed reload".into());
    }
    let states: Vec<_> = file
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclClothState")
        .collect();
    if states.len() != state_usage.len() {
        return Err("Starfield cloth reload changed state count".into());
    }
    for (actual, expected) in states.iter().zip(state_usage) {
        for expected in expected {
            if !same_usage_value(&expected.value, field(&actual.members, &expected.name)?) {
                return Err(format!(
                    "Starfield cloth reload changed state {} bitsets/indices",
                    expected.name
                ));
            }
        }
    }
    let skeleton = &file.objects()[unique(file, "hkaSkeleton")?].members;
    let reloaded_parents = array(field(skeleton, "parentIndices")?)?
        .iter()
        .map(integer)
        .collect::<Result<Vec<_>, _>>()?;
    if reloaded_parents != parents
        || array(field(skeleton, "bones")?)?.len() != worlds.len()
        || usage_indices(file, worlds.len())? != *used
    {
        return Err("Starfield cloth reload changed skeleton/usage indices".into());
    }
    for (bone, name) in array(field(skeleton, "bones")?)?.iter().zip(target_names) {
        if !matches!(field(members(bone)?, "name")?, HkxValue::String{value,..} if value==name) {
            return Err("Starfield cloth reload changed target bone names".into());
        }
    }
    let mut actual = Vec::new();
    for (i, pose) in array(field(skeleton, "referencePose")?)?.iter().enumerate() {
        let local = qs_matrix(pose, 1.)?;
        let world = if parents[i] < 0 {
            local
        } else {
            mat4_mul(&actual[parents[i] as usize], &local)
        };
        if residual(&world, &worlds[i]) > 1e-3 {
            return Err(format!(
                "Starfield cloth target rest failed reload at bone {i}"
            ));
        }
        actual.push(world);
    }
    let pose = &file.objects()[unique(file, "hclSimClothPose")?].members;
    let reloaded_points = point_values(field(pose, "positions")?)?;
    if reloaded_points.len() != points.len()
        || reloaded_points
            .iter()
            .zip(points)
            .any(|(a, b)| (0..4).any(|j| (a[j] - b[j]).abs() > 2e-4))
    {
        return Err("Starfield cloth proxy pose failed reload".into());
    }
    let op = &file.objects()[skin_operator(file)?];
    let count = skin_vertex_count(file, op)?;
    let vertices = particle_vertices(file, points.len(), count)?;
    let evaluated = proxy_positions(op, &actual, count, 1.)?;
    if evaluated.is_empty()
        || vertices.iter().zip(points).any(|(v, b)| {
            let expected = canonical_proxy.map(|p| &p[*v]).unwrap_or(b);
            (0..3).any(|j| (evaluated[*v][j] - expected[j]).abs() > 1e-3)
        })
    {
        return Err("Starfield cloth skin/proxy rest disagrees after reload".into());
    }
    let deform = &file.objects()[unique(file, "hclSimpleMeshBoneDeformOperator")?].members;
    let pairs = array(field(deform, "triangleBonePairs")?)?;
    let matrices = array(field(deform, "localBoneTransforms")?)?;
    if pairs != expected_pairs || matrices.len() != pairs.len() {
        return Err("Starfield cloth reload changed deformation offsets/counts".into());
    }
    for (pair, matrix) in pairs.iter().zip(matrices) {
        let p = members(pair)?;
        let bone = index(field(p, "boneOffset")?)? / 64;
        let f = triangle_frame(
            &reloaded_points,
            triangles,
            index(field(p, "triangleOffset")?)?,
        )?;
        if residual(&mat4_mul(&f, &value_matrix(matrix)?), &worlds[bone]) > 1e-3 {
            return Err(format!(
                "Starfield cloth deformed bone rest failed reload at {bone}"
            ));
        }
    }
    Ok(())
}

fn same_usage_value(a: &HkxValue, b: &HkxValue) -> bool {
    if let (Some(a), Some(b)) = (a.as_object_members(), b.as_object_members()) {
        return a.len() == b.len()
            && a.iter()
                .all(|m| field(b, &m.name).is_ok_and(|v| same_usage_value(&m.value, v)));
    }
    if let (HkxValue::Array(a), HkxValue::Array(b)) = (a, b) {
        return a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_usage_value(a, b));
    }
    if let (Ok(a), Ok(b)) = (integer(a), integer(b)) {
        return a == b;
    }
    a == b
}

fn anchor_canonical_inputs(file: &mut HkxFile, corrections: &[BindMatrix]) -> Result<(), String> {
    for object in file.objects_mut() {
        let m = &mut object.members;
        match object.class_name.as_str() {
            "hclObjectSpaceSkinPNOperator" => {
                let subset: Vec<_> = array(field(m, "transformSubset")?)?
                    .iter()
                    .map(index)
                    .collect::<Result<_, _>>()?;
                let ids: Vec<_> = if subset.is_empty() {
                    (0..corrections.len()).collect()
                } else {
                    subset
                };
                let HkxValue::Array(binds) = field_mut(m, "boneFromSkinMeshTransforms")? else {
                    unreachable!()
                };
                if binds.len() != ids.len() {
                    return Err("Starfield canonical cloth bind/subset counts disagree".into());
                }
                for (bind, bone) in binds.iter_mut().zip(ids) {
                    let correction = corrections
                        .get(bone)
                        .ok_or("Starfield canonical cloth bind index out of range")?;
                    *bind = matrix_value(&mat4_mul(correction, &value_matrix(bind)?));
                }
            }
            "hclBoneSpaceSkinPNOperator" => {
                return Err(
                    "unsupported Starfield canonical BoneSpace packed-normal retargeting".into(),
                );
            }
            "hclBonePlanesConstraintSet" => {
                let HkxValue::Array(planes) = field_mut(m, "bonePlanes")? else {
                    unreachable!()
                };
                for plane in planes {
                    let p = plane
                        .as_object_members_mut()
                        .ok_or("Starfield canonical cloth invalid bone plane")?;
                    let correction = corrections
                        .get(index(field(p, "transformIndex")?)?)
                        .ok_or("Starfield canonical cloth plane bone out of range")?;
                    let eq = floats(field(p, "planeEquationBone")?)?;
                    let normal: [f64; 3] = std::array::from_fn(|row| {
                        (0..3).map(|col| correction[row][col] * eq[col]).sum()
                    });
                    let offset = eq[3]
                        - (0..3)
                            .map(|row| normal[row] * correction[row][3])
                            .sum::<f64>();
                    *field_mut(p, "planeEquationBone")? = HkxValue::F32List(vec![
                        normal[0] as f32,
                        normal[1] as f32,
                        normal[2] as f32,
                        offset as f32,
                    ]);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn lower_marked_bend_mx(file: &mut HkxFile, particles: usize) -> Result<(), String> {
    for object in file
        .objects_mut()
        .iter_mut()
        .filter(|o| o.class_name == "hclBendStiffnessConstraintSetMx")
    {
        let m = &mut object.members;
        if field(m, "clampBendStiffness")? != &HkxValue::Bool(true)
            || field(m, "useRestPoseConfig")? != &HkxValue::Bool(true)
        {
            return Err("Starfield BendMx v1 requires authored clamped rest-pose solver".into());
        }
        let HkxValue::F32(cutoff) = field(m, "maxRestPoseHeightSq")? else {
            return Err("Starfield BendMx invalid cutoff".into());
        };
        let cutoff = (*cutoff as f64 * UNIT_SCALE.powi(2)) as f32;
        if !cutoff.is_finite() || cutoff <= 0. {
            return Err("Starfield BendMx cutoff must be finite and positive".into());
        }
        let HkxValue::String {
            value: original,
            is_null: false,
        } = field(m, "name")?
        else {
            return Err("Starfield BendMx requires authored constraint name".into());
        };
        if original.starts_with("B21_SF_BENDMX:") {
            return Err("Starfield BendMx source uses reserved runtime marker".into());
        }
        let name = format!("{BEND_MX_MARKER}{:08X};{original}", cutoff.to_bits());
        *field_mut(m, "name")? = HkxValue::String {
            value: name,
            is_null: false,
        };
        let HkxValue::Array(batches) = field_mut(m, "batches")? else {
            unreachable!()
        };
        for batch in batches {
            for member in batch
                .as_object_members_mut()
                .ok_or("Starfield BendMx invalid batch")?
            {
                let HkxValue::Array(values) = &mut member.value else {
                    return Err("Starfield BendMx invalid batch member".into());
                };
                if values.len() != 4 {
                    return Err("Starfield BendMx requires source four-lane batches".into());
                }
                let inert = match member.name.as_str() {
                    "particlesA" | "particlesB" | "particlesC" | "particlesD" => {
                        if values
                            .iter()
                            .any(|v| index(v).map_or(true, |i| i >= particles))
                        {
                            return Err("Starfield BendMx particle index out of range".into());
                        }
                        values[0].clone()
                    }
                    "weightsA" | "weightsB" | "weightsC" | "weightsD" | "bendStiffnesses"
                    | "restCurvatures" | "invMassesA" | "invMassesB" | "invMassesC"
                    | "invMassesD" => {
                        for value in values.iter_mut() {
                            if member.name.starts_with("invMasses")
                                && !matches!(value, HkxValue::F32(v) if v.is_finite() && *v >= 0.)
                            {
                                return Err("Starfield BendMx invalid inverse mass".into());
                            }
                            scaled_number(
                                value,
                                if member.name == "restCurvatures" {
                                    1. / UNIT_SCALE
                                } else {
                                    1.
                                },
                            )?;
                        }
                        if member.name == "bendStiffnesses" {
                            HkxValue::F32(0.)
                        } else {
                            values[0].clone()
                        }
                    }
                    _ => {
                        return Err(format!(
                            "unsupported Starfield BendMx batch field {}",
                            member.name
                        ))
                    }
                };
                values.resize(16, inert);
            }
        }
        let HkxValue::Array(singles) = field_mut(m, "singles")? else {
            unreachable!()
        };
        for single in singles {
            for member in single
                .as_object_members_mut()
                .ok_or("Starfield BendMx invalid single")?
            {
                match member.name.as_str() {
                    "particleA" | "particleB" | "particleC" | "particleD" => {
                        if index(&member.value)? >= particles {
                            return Err("Starfield BendMx particle index out of range".into());
                        }
                    }
                    "weightA" | "weightB" | "weightC" | "weightD" | "bendStiffness"
                    | "restCurvature" | "invMassA" | "invMassB" | "invMassC" | "invMassD" => {
                        if member.name.starts_with("invMass")
                            && !matches!(&member.value, HkxValue::F32(v) if v.is_finite() && *v >= 0.)
                        {
                            return Err("Starfield BendMx invalid inverse mass".into());
                        }
                        scaled_number(
                            &mut member.value,
                            if member.name == "restCurvature" {
                                1. / UNIT_SCALE
                            } else {
                                1.
                            },
                        )?;
                    }
                    _ => {
                        return Err(format!(
                            "unsupported Starfield BendMx single field {}",
                            member.name
                        ))
                    }
                }
            }
        }
        m.retain(|member| {
            !matches!(
                member.name.as_str(),
                "clampBendStiffness" | "maxRestPoseHeightSq"
            )
        });
    }
    Ok(())
}

pub(crate) fn adapt_starfield_cloth(
    blob: &[u8],
    target_skeleton: &NifFile,
    map: &SkeletonMap,
) -> Result<AdaptedCloth, String> {
    adapt_starfield_cloth_with_capabilities(
        blob,
        target_skeleton,
        map,
        &ClothRuntimeCapabilities::default(),
    )
}

pub(crate) fn adapt_starfield_cloth_with_capabilities(
    blob: &[u8],
    target_skeleton: &NifFile,
    map: &SkeletonMap,
    capabilities: &ClothRuntimeCapabilities,
) -> Result<AdaptedCloth, String> {
    let tag = hkx::parse_tagfile(blob).map_err(|e| e.to_string())?;
    if tag.sdk_version != "20190200" {
        return Err(format!(
            "unsupported Starfield cloth SDK {}",
            tag.sdk_version
        ));
    }
    let mut file = tag.materialize_hkx().map_err(|e| e.to_string())?;
    check_classes_with_capabilities(&file, capabilities)?;
    let canonical = file
        .objects()
        .iter()
        .any(|o| o.class_name == "hclBendStiffnessConstraintSetMx");
    let state_usage: Vec<_> = file
        .objects()
        .iter()
        .filter(|o| o.class_name == "hclClothState")
        .map(|o| {
            o.members
                .iter()
                .filter(|m| {
                    matches!(
                        m.name.as_str(),
                        "operators" | "usedBuffers" | "usedTransformSets" | "usedSimCloths"
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    let skel_id = unique(&file, "hkaSkeleton")?;
    let transform_id = unique(&file, "hclTransformSetDefinition")?;
    let sim_id = unique(&file, "hclSimClothData")?;
    let pose_id = unique(&file, "hclSimClothPose")?;
    let skin_id = skin_operator(&file)?;
    let deform_id = unique(&file, "hclSimpleMeshBoneDeformOperator")?;
    let skeleton = &file.objects()[skel_id].members;
    let names = array(field(skeleton, "bones")?)?
        .iter()
        .map(|bone| match field(members(bone)?, "name")? {
            HkxValue::String { value, .. } if !value.is_empty() => Ok(value.clone()),
            _ => Err("Starfield cloth bone has no name".into()),
        })
        .collect::<Result<Vec<String>, String>>()?;
    let parents = array(field(skeleton, "parentIndices")?)?
        .iter()
        .map(integer)
        .collect::<Result<Vec<_>, _>>()?;
    let poses = array(field(skeleton, "referencePose")?)?.to_vec();
    if names.is_empty()
        || parents.len() != names.len()
        || poses.len() != names.len()
        || index(field(
            &file.objects()[transform_id].members,
            "numTransforms",
        )?)? != names.len()
    {
        return Err("Starfield cloth skeleton/transform counts disagree".into());
    }
    let mut source_worlds = Vec::new();
    for (i, p) in poses.iter().enumerate() {
        let local = qs_matrix(p, 1.)?;
        source_worlds.push(if parents[i] < 0 {
            local
        } else {
            let parent = parents[i] as usize;
            if parent >= i {
                return Err("Starfield cloth skeleton is not parent ordered".into());
            }
            mat4_mul(&source_worlds[parent], &local)
        });
    }
    let used = usage_indices(&file, names.len())?;
    for object in file.objects() {
        if object.class_name == "hclBonePlanesConstraintSet" {
            for plane in array(field(&object.members, "bonePlanes")?)? {
                let bone = index(field(members(plane)?, "transformIndex")?)?;
                if !used.contains(&bone) {
                    return Err("Starfield cloth bone plane missing usage binding".into());
                }
            }
        }
    }
    let deform = &file.objects()[deform_id].members;
    if index(field(deform, "outputTransformSetIdx")?)? != 0 {
        return Err("unsupported Starfield cloth deform transform set".into());
    }
    let pairs = array(field(deform, "triangleBonePairs")?)?.to_vec();
    let old_matrices = array(field(deform, "localBoneTransforms")?)?.to_vec();
    if pairs.is_empty() || pairs.len() != old_matrices.len() {
        return Err("Starfield cloth empty/inconsistent deformation palette".into());
    }
    let mut custom = BTreeSet::new();
    for pair in &pairs {
        let offset = index(field(members(pair)?, "boneOffset")?)?;
        if offset % 64 != 0 || offset / 64 >= names.len() {
            return Err("Starfield cloth invalid bone offset".into());
        }
        custom.insert(offset / 64);
    }
    let target = target_worlds(target_skeleton)?;
    let mut required_custom = custom.clone();
    for &bone in &custom {
        let mut i = bone;
        while parents[i] >= 0 {
            i = parents[i] as usize;
            if let Some(mapped) = map.lookup(&names[i]) {
                if !target.contains_key(&mapped.to_ascii_lowercase()) {
                    return Err(format!(
                        "Starfield cloth target is missing {mapped} for {}",
                        names[i]
                    ));
                }
                break;
            }
            required_custom.insert(i);
        }
    }
    let mut worlds: Vec<BindMatrix> = Vec::new();
    let mut target_names: Vec<String> = Vec::new();
    let mut bindings = Vec::new();
    let mut unbound_unused = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let parent = if parents[i] < 0 {
            None
        } else {
            Some(parents[i] as usize)
        };
        let parent_world = parent.map(|p| worlds[p]).unwrap_or_else(identity);
        let mapped = map
            .lookup(name)
            .and_then(|n| target.get(&n.to_ascii_lowercase()));
        let (target_name, world, node) = if let Some((target_name, world)) = mapped {
            if custom.contains(&i) {
                return Err(format!("Starfield cloth deforms structural bone {name}"));
            }
            (target_name.clone(), *world, None)
        } else if required_custom.contains(&i) {
            let p = parent.ok_or("Starfield cloth custom root is unsupported")?;
            if target.contains_key(&name.to_ascii_lowercase()) {
                return Err(format!(
                    "Starfield cloth custom bone name collides with target {name}"
                ));
            }
            let local = qs_matrix(&poses[i], UNIT_SCALE)?;
            (
                name.clone(),
                mat4_mul(&parent_world, &local),
                Some(ClothBoneNode {
                    parent: target_names[p].clone(),
                    local,
                }),
            )
        } else {
            if used.contains(&i) {
                return Err(format!("unmapped Starfield cloth input bone {name}"));
            }
            unbound_unused.push(name.clone());
            let world = mat4_mul(&parent_world, &qs_matrix(&poses[i], UNIT_SCALE)?);
            worlds.push(world);
            target_names.push(name.clone());
            continue;
        };
        worlds.push(world);
        target_names.push(target_name.clone());
        bindings.push(ClothBoneBinding {
            source_name: name.clone(),
            target_name,
            world,
            custom: node,
        });
    }
    let source_points = point_values(field(&file.objects()[pose_id].members, "positions")?)?;
    let skin_count = skin_vertex_count(&file, &file.objects()[skin_id])?;
    let vertices = particle_vertices(&file, source_points.len(), skin_count)?;
    for object in file.objects() {
        let m = &object.members;
        match object.class_name.as_str() {
            "hclMoveParticlesOperator" => {
                if index(field(m, "simClothIndex")?)? != 0 || index(field(m, "refBufferIdx")?)? != 0
                {
                    return Err("unsupported Starfield cloth move buffer contract".into());
                }
                for pair in array(field(m, "vertexParticlePairs")?)? {
                    let pair = members(pair)?;
                    if vertices.get(index(field(pair, "particleIndex")?)?)
                        != Some(&index(field(pair, "vertexIndex")?)?)
                    {
                        return Err("unsupported Starfield cloth proxy particle remapping".into());
                    }
                }
            }
            "hclCopyVerticesOperator" => {
                if index(field(m, "inputBufferIdx")?)? != 0
                    || index(field(m, "outputBufferIdx")?)? != 1
                    || index(field(m, "numberOfVertices")?)? != source_points.len()
                    || index(field(m, "startVertexIn")?)? != 0
                    || index(field(m, "startVertexOut")?)? != 0
                {
                    return Err("unsupported Starfield cloth copy buffer contract".into());
                }
            }
            "hclBoneSpaceSkinPNOperator" | "hclObjectSpaceSkinPNOperator"
                if index(field(m, "outputBufferIndex")?)? != 0 =>
            {
                return Err("unsupported Starfield cloth skin output buffer".into());
            }
            "hclSimpleMeshBoneDeformOperator" if index(field(m, "inputBufferIdx")?)? != 1 => {
                return Err("unsupported Starfield cloth deform input buffer".into());
            }
            "hclSimulateOperator" if index(field(m, "simClothIndex")?)? != 0 => {
                return Err("unsupported Starfield cloth simulation index".into());
            }
            _ => {}
        }
    }
    let evaluated = proxy_positions(&file.objects()[skin_id], &source_worlds, skin_count, 1.)?;
    let quantization_bound = if file.objects()[skin_id].class_name == "hclObjectSpaceSkinPNOperator"
    {
        object_proxy_positions(&file.objects()[skin_id], &source_worlds, skin_count, 1.)?.1
    } else {
        0.
    };
    if source_points
        .iter()
        .zip(&vertices)
        .any(|(a, v)| (0..3).any(|j| (a[j] - evaluated[*v][j]).abs() > quantization_bound + 2e-5))
    {
        return Err("unsupported Starfield cloth proxy coordinate/binding contract".into());
    }
    let target_skin_points =
        proxy_positions(&file.objects()[skin_id], &worlds, skin_count, UNIT_SCALE)?;
    let target_points: Vec<_> = if canonical {
        source_points
            .iter()
            .map(|p| {
                [
                    p[0] * UNIT_SCALE,
                    p[1] * UNIT_SCALE,
                    p[2] * UNIT_SCALE,
                    p[3],
                ]
            })
            .collect()
    } else {
        vertices.iter().map(|v| target_skin_points[*v]).collect()
    };
    let canonical_corrections = if canonical {
        Some(
            source_worlds
                .iter()
                .zip(&worlds)
                .map(|(source, target)| {
                    let mut scaled = *source;
                    for row in 0..3 {
                        scaled[row][3] *= UNIT_SCALE;
                    }
                    Ok(mat4_mul(
                        &mat4_affine_inverse(target)
                            .ok_or("Starfield cloth singular canonical target")?,
                        &scaled,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )
    } else {
        None
    };
    let triangles = array(field(&file.objects()[sim_id].members, "triangleIndices")?)?
        .iter()
        .map(index)
        .collect::<Result<Vec<_>, _>>()?;
    let mut matrices = Vec::new();
    for (pair, old) in pairs.iter().zip(&old_matrices) {
        let pair = members(pair)?;
        let bone = index(field(pair, "boneOffset")?)? / 64;
        let offset = index(field(pair, "triangleOffset")?)?;
        if residual(
            &mat4_mul(
                &triangle_frame(&source_points, &triangles, offset)?,
                &value_matrix(old)?,
            ),
            &source_worlds[bone],
        ) > 2e-4
        {
            return Err("unsupported Starfield cloth authored deformation rest contract".into());
        }
        let f = triangle_frame(&target_points, &triangles, offset)?;
        matrices.push(matrix_value(&mat4_mul(
            &mat4_affine_inverse(&f).unwrap(),
            &worlds[bone],
        )));
    }
    let new_poses = worlds
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let local = if parents[i] < 0 {
                *w
            } else {
                mat4_mul(
                    &mat4_affine_inverse(&worlds[parents[i] as usize])
                        .ok_or("Starfield cloth singular target parent")?,
                    w,
                )
            };
            matrix_qs(&local)
        })
        .collect::<Result<Vec<_>, String>>()?;
    *field_mut(&mut file.objects_mut()[skel_id].members, "referencePose")? =
        HkxValue::Array(new_poses);
    let HkxValue::Array(bones) = field_mut(&mut file.objects_mut()[skel_id].members, "bones")?
    else {
        unreachable!()
    };
    for (bone, name) in bones.iter_mut().zip(&target_names) {
        *field_mut(bone.as_object_members_mut().unwrap(), "name")? = HkxValue::String {
            value: name.clone(),
            is_null: false,
        };
    }
    *field_mut(
        &mut file.objects_mut()[deform_id].members,
        "localBoneTransforms",
    )? = HkxValue::Array(matrices);
    *field_mut(&mut file.objects_mut()[pose_id].members, "positions")? = HkxValue::Array(
        target_points
            .iter()
            .map(|p| HkxValue::F32List(p.iter().map(|v| *v as f32).collect()))
            .collect(),
    );
    adapt_collidables(
        &mut file,
        &source_worlds,
        &worlds,
        &used,
        canonical_corrections.as_deref(),
    )?;
    adapt_known_fields(&mut file, &source_points, &target_points, skin_count)?;
    if let Some(corrections) = &canonical_corrections {
        anchor_canonical_inputs(&mut file, corrections)?;
        lower_marked_bend_mx(&mut file, source_points.len())?;
    }
    let expected_gravity = floats(field(
        members(field(&file.objects()[sim_id].members, "simulationInfo")?)?,
        "gravity",
    )?)?;
    let lowered =
        migrate_2015_packfile_to_2014_with_warnings(file, Fo76MigrationOptions::default())
            .map_err(|e| e.to_string())?;
    if !lowered.warnings.is_empty() {
        return Err(format!(
            "Starfield cloth lowering warnings: {:?}",
            lowered.warnings
        ));
    }
    let blob = hkx::write_hkx(
        &lowered.hkx,
        &mut DescriptorRegistry::for_contents_version("hk_2014.1.0-r1"),
    );
    let reloaded = HkxFile::read(&blob).map_err(|e| e.to_string())?;
    let canonical_proxy: Option<Vec<_>> = canonical.then(|| {
        evaluated
            .iter()
            .map(|p| {
                [
                    p[0] * UNIT_SCALE,
                    p[1] * UNIT_SCALE,
                    p[2] * UNIT_SCALE,
                    p[3],
                ]
            })
            .collect()
    });
    validate_adapted(
        &reloaded,
        &parents,
        &target_points,
        &triangles,
        &worlds,
        &used,
        &pairs,
        &target_names,
        &state_usage,
        canonical_proxy.as_deref(),
    )?;
    let sim = &reloaded.objects()[unique(&reloaded, "hclSimClothData")?].members;
    if floats(field(members(field(sim, "simulationInfo")?)?, "gravity")?)? != expected_gravity {
        return Err("Starfield cloth gravity failed reload".into());
    }
    Ok(AdaptedCloth {
        blob,
        bones: bindings,
        unbound_unused,
        runtime_requirements: if canonical {
            vec![BEND_MX_CAPABILITY]
        } else {
            vec![]
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use havok_native::api;
    use serde_json::{json, Value};

    #[test]
    fn deformation_frame_preserves_centroid_and_unnormalized_edges() {
        let p = [[0., 0., 0., 1.], [6., 0., 0., 1.], [0., 3., 0., 1.]];
        let f = triangle_frame(&p, &[0, 1, 2], 0).unwrap();
        assert_eq!(
            f,
            [
                [-2., 4., 0., 2.],
                [-1., -1., 0., 1.],
                [0., 0., 6., 0.],
                [0., 0., 0., 1.]
            ]
        );
        let mut desired = identity();
        desired[0][3] = 4.;
        desired[1][3] = 7.;
        desired[2][3] = 2.;
        let local = mat4_mul(&mat4_affine_inverse(&f).unwrap(), &desired);
        assert!(residual(&mat4_mul(&f, &local), &desired) < 1e-12);
        assert!(triangle_frame(&p, &[0, 0, 0], 0).is_err());
    }

    #[test]
    fn unknown_cloth_operator_fails_instead_of_being_dropped() {
        let file = HkxFile::from_tagxml(
            11,
            "hk_2019.0.0-r1",
            vec![HkxObject {
                name: None,
                offset: 0,
                signature: 0,
                class_name: "hclUnknownOperator".into(),
                members: vec![],
            }],
        );
        assert!(check_classes(&file)
            .unwrap_err()
            .contains("hclUnknownOperator"));
    }

    #[test]
    #[ignore = "requires extracted Starfield cloth cohort"]
    fn bend_mx_cohort_retains_clamped_solver_failure() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let receipt: Value = serde_json::from_slice(
            &std::fs::read(std::env::temp_dir().join("b21_starfield_cloth_cohort/receipt.json"))
                .unwrap(),
        )
        .unwrap();
        let paths: BTreeSet<_> = receipt["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                r["source_classes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c == "hclBendStiffnessConstraintSetMx")
            })
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(paths.len(), 62);
        let scalar = |m: &[HkxMember], name| match field(m, name).unwrap() {
            HkxValue::F32(value) => *value as f64,
            _ => panic!("invalid bend scalar {name}"),
        };
        let mut rows = Vec::new();
        for path in paths {
            let bytes = std::fs::read(repo.join("extracted/starfield/meshes").join(&path)).unwrap();
            for blob in crate::cloth::extract_cloth_blobs(&bytes).unwrap() {
                let file = hkx::parse_tagfile(&blob)
                    .unwrap()
                    .materialize_hkx()
                    .unwrap();
                let bones = array(
                    field(
                        &file.objects()[unique(&file, "hkaSkeleton").unwrap()].members,
                        "bones",
                    )
                    .unwrap(),
                )
                .unwrap()
                .len();
                usage_indices(&file, bones).unwrap();
                for object in file
                    .objects()
                    .iter()
                    .filter(|o| o.class_name == "hclBendStiffnessConstraintSetMx")
                {
                    let m = &object.members;
                    let singles = array(field(m, "singles").unwrap()).unwrap();
                    let batches = array(field(m, "batches").unwrap()).unwrap();
                    let object_index = file
                        .objects()
                        .iter()
                        .position(|o| std::ptr::eq(o, object))
                        .unwrap();
                    let sim: Vec<_> = file
                        .objects()
                        .iter()
                        .filter(|o| {
                            o.class_name == "hclSimClothData"
                                && array(field(&o.members, "staticConstraintSets").unwrap())
                                    .unwrap()
                                    .contains(&HkxValue::Pointer(Some(object_index)))
                        })
                        .collect();
                    assert_eq!(sim.len(), 1);
                    let particle_count = array(field(&sim[0].members, "particleDatas").unwrap())
                        .unwrap()
                        .len();
                    for single in singles {
                        for name in ["particleA", "particleB", "particleC", "particleD"] {
                            assert!(
                                index(field(members(single).unwrap(), name).unwrap()).unwrap()
                                    < particle_count
                            );
                        }
                    }
                    for batch in batches {
                        for name in ["particlesA", "particlesB", "particlesC", "particlesD"] {
                            let indices =
                                array(field(members(batch).unwrap(), name).unwrap()).unwrap();
                            assert_eq!(indices.len(), 4);
                            assert!(indices.iter().all(|i| index(i).unwrap() < particle_count));
                        }
                    }
                    let curved = singles
                        .iter()
                        .filter(|single| scalar(members(single).unwrap(), "restCurvature") != 0.)
                        .count()
                        + batches
                            .iter()
                            .map(|batch| {
                                array(field(members(batch).unwrap(), "restCurvatures").unwrap())
                                    .unwrap()
                                    .iter()
                                    .filter(|v| **v != HkxValue::F32(0.))
                                    .count()
                            })
                            .sum::<usize>();
                    rows.push(
                        json!({"path":path,"batches":batches.len(),"singles":singles.len(),
                        "curved_constraints":curved,
                        "clamp":field(m,"clampBendStiffness").unwrap()==&HkxValue::Bool(true),
                        "rest_pose":field(m,"useRestPoseConfig").unwrap()==&HkxValue::Bool(true),
                        "max_height_sq":scalar(m,"maxRestPoseHeightSq")}),
                    );
                }
            }
        }
        let output = std::env::temp_dir().join("b21_starfield_bend_mx_audit.json");
        std::fs::write(&output, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
        eprintln!(
            "Starfield bend MX audit: {} constraint sets; {}",
            rows.len(),
            output.display()
        );
        assert_eq!(rows.len(), 74);
        assert!(rows
            .iter()
            .all(|r| r["clamp"] == true && r["rest_pose"] == true));
        assert!(rows
            .iter()
            .all(|r| r["curved_constraints"].as_u64().unwrap() > 0));
    }

    #[test]
    #[ignore = "requires actual Akila Starfield cloth"]
    fn authored_bend_mx_clamp_cannot_be_dropped_for_fo4() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = std::fs::read(repo.join("extracted/starfield/meshes/clothes/outfit_akila_security/outfit_akila_security_upperbody_m.nif")).unwrap();
        let blob = crate::cloth::extract_cloth_blob(&bytes).unwrap();
        let file = hkx::parse_tagfile(&blob)
            .unwrap()
            .materialize_hkx()
            .unwrap();
        let bend =
            &file.objects()[unique(&file, "hclBendStiffnessConstraintSetMx").unwrap()].members;
        assert_eq!(
            field(bend, "clampBendStiffness").unwrap(),
            &HkxValue::Bool(true)
        );
        assert_eq!(
            field(bend, "useRestPoseConfig").unwrap(),
            &HkxValue::Bool(true)
        );
        let scalar = |m: &[HkxMember], name| match field(m, name).unwrap() {
            HkxValue::F32(value) => *value as f64,
            _ => panic!("invalid bend scalar {name}"),
        };
        let pose = &file.objects()[unique(&file, "hclSimClothPose").unwrap()].members;
        let points = point_values(field(pose, "positions").unwrap()).unwrap();
        let single = members(&array(field(bend, "singles").unwrap()).unwrap()[0]).unwrap();
        let mut p: Vec<_> = ["particleA", "particleB", "particleC", "particleD"]
            .iter()
            .map(|name| {
                let point = points[index(field(single, name).unwrap()).unwrap()];
                [point[0], point[1], point[2]]
            })
            .collect();
        let sub = |a: [f64; 3], b: [f64; 3]| std::array::from_fn::<_, 3, _>(|i| a[i] - b[i]);
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let length = |a: [f64; 3]| a.iter().map(|x| x * x).sum::<f64>().sqrt();
        let edge = sub(p[3], p[2]);
        let rest_height = length(cross(edge, sub(p[0], p[2])))
            * length(cross(sub(p[1], p[2]), edge))
            / length(edge).powi(2)
            * scalar(single, "restCurvature");
        assert!(rest_height.abs() > 0.);
        let scale = (2. * scalar(bend, "maxRestPoseHeightSq").sqrt() / rest_height.abs()).sqrt();
        for point in &mut p {
            for value in point {
                *value *= scale;
            }
        }
        let edge = sub(p[3], p[2]);
        let a = cross(edge, sub(p[0], p[2]));
        let b = cross(sub(p[1], p[2]), edge);
        let height = length(a) * length(b) / length(edge).powi(2) * scalar(single, "restCurvature");
        assert!(height.powi(2) > scalar(bend, "maxRestPoseHeightSq"));
        let normal: [f64; 3] = std::array::from_fn(|i| a[i] / length(a) + b[i] / length(b));
        let weights: Vec<_> = ["weightA", "weightB", "weightC", "weightD"]
            .iter()
            .map(|name| scalar(single, name))
            .collect();
        let correction: [f64; 3] = std::array::from_fn(|i| {
            let delta = (0..4).map(|j| p[j][i] * weights[j]).sum::<f64>()
                + normal[i] / length(normal) * height;
            scalar(single, "bendStiffness") * weights[0] * scalar(single, "invMassA") * delta
        });
        assert!(correction.iter().all(|v| v.is_finite()));
        assert!(length(correction) > 1e-4);
        assert!(check_classes(&file)
            .unwrap_err()
            .contains("hclBendStiffnessConstraintSetMx"));
    }

    fn bend_length(v: [f64; 3]) -> f64 {
        v.iter().map(|x| x * x).sum::<f64>().sqrt()
    }

    fn bend_sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| a[i] - b[i])
    }

    fn bend_cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    fn bend_shape(p: &[[f64; 3]; 4]) -> ([f64; 3], f64) {
        let edge = bend_sub(p[3], p[2]);
        let a = bend_cross(edge, bend_sub(p[0], p[2]));
        let b = bend_cross(bend_sub(p[1], p[2]), edge);
        let sum = std::array::from_fn(|i| a[i] / bend_length(a) + b[i] / bend_length(b));
        (
            sum.map(|v| v / bend_length(sum)),
            bend_length(a) * bend_length(b) / bend_length(edge).powi(2),
        )
    }

    fn cook_bend(p: &[[f64; 3]; 4]) -> ([f64; 4], f64) {
        let area = |a, b, c| bend_length(bend_cross(bend_sub(p[a], p[b]), bend_sub(p[a], p[c])));
        let a = area(0, 3, 2);
        let b = area(1, 2, 3);
        let c = area(2, 0, 1);
        let d = area(3, 1, 0);
        let weights = [b / (a + b), a / (a + b), -d / (c + d), -c / (c + d)];
        let (normal, metric) = bend_shape(p);
        let residual: [f64; 3] =
            std::array::from_fn(|i| (0..4).map(|j| weights[j] * p[j][i]).sum::<f64>());
        (
            weights,
            -(0..3).map(|i| residual[i] * normal[i]).sum::<f64>() / metric,
        )
    }

    #[cfg(target_arch = "x86_64")]
    fn cook_bend_primary_f32(p: &[[f64; 3]; 4]) -> ([f32; 4], f32) {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        fn inverse_length(v: [f32; 3]) -> (f32, f32) {
            let square = (v[0] * v[0] + v[1] * v[1]) + v[2] * v[2];
            let estimate = unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(square))) };
            let inverse = if square > 0. {
                (3. - (estimate * square) * estimate) * (estimate * 0.5)
            } else {
                0.
            };
            (inverse, square)
        }
        let p = p.map(|v| v.map(|x| x as f32));
        let sub = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| a[i] - b[i]);
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let normals = [
            cross(sub(p[0], p[2]), sub(p[0], p[3])),
            cross(sub(p[1], p[3]), sub(p[1], p[2])),
            cross(sub(p[2], p[0]), sub(p[2], p[1])),
            cross(sub(p[3], p[1]), sub(p[3], p[0])),
        ];
        let inverse = normals.map(inverse_length);
        let area: [f32; 4] = std::array::from_fn(|i| inverse[i].0 * inverse[i].1);
        let ab = 1. / (area[0] + area[1]);
        let cd = 1. / (area[2] + area[3]);
        let weights = [ab * area[1], ab * area[0], -cd * area[3], -cd * area[2]];
        let sum: [f32; 3] =
            std::array::from_fn(|i| normals[0][i] * inverse[0].0 + normals[1][i] * inverse[1].0);
        let normalize = inverse_length(sum).0;
        let weighted: [f32; 3] = std::array::from_fn(|i| {
            (((weights[0] * p[0][i]) + (weights[1] * p[1][i])) + (weights[2] * p[2][i]))
                + (weights[3] * p[3][i])
        });
        let dot: [f32; 3] = std::array::from_fn(|i| weighted[i] * (normalize * sum[i]));
        let edge = sub(p[3], p[2]);
        let edge_square = (edge[0] * edge[0] + edge[1] * edge[1]) + edge[2] * edge[2];
        (
            weights,
            -((dot[0] + dot[1]) + dot[2]) / ((area[0] * area[1]) / edge_square),
        )
    }

    fn bend_correction(
        p: &[[f64; 3]; 4],
        weights: &[f64; 4],
        curvature: f64,
        stiffness: f64,
        inv_mass: &[f64; 4],
        max_height_sq: f64,
    ) -> [[f64; 3]; 4] {
        let (normal, metric) = bend_shape(p);
        let height = metric * curvature;
        let stiffness = if height * height > max_height_sq {
            0.
        } else {
            stiffness
        };
        let delta: [f64; 3] = std::array::from_fn(|i| {
            (0..4).map(|j| weights[j] * p[j][i]).sum::<f64>() + normal[i] * height
        });
        std::array::from_fn(|j| delta.map(|v| v * weights[j] * stiffness * inv_mass[j]))
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    #[ignore = "requires actual Akila Starfield cloth and Fallout4 target skeleton"]
    fn authored_bend_cooking_and_canonical_proxy_anchor_preserve_source_metric() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = std::fs::read(repo.join("extracted/starfield/meshes/clothes/outfit_akila_security/outfit_akila_security_upperbody_m.nif")).unwrap();
        let blob = crate::cloth::extract_cloth_blob(&bytes).unwrap();
        let file = hkx::parse_tagfile(&blob)
            .unwrap()
            .materialize_hkx()
            .unwrap();
        let target = NifFile::load(
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        )
        .unwrap();
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let skeleton = &file.objects()[unique(&file, "hkaSkeleton").unwrap()].members;
        let parents: Vec<_> = array(field(skeleton, "parentIndices").unwrap())
            .unwrap()
            .iter()
            .map(|v| integer(v).unwrap())
            .collect();
        let names: Vec<_> = array(field(skeleton, "bones").unwrap())
            .unwrap()
            .iter()
            .map(|v| match field(members(v).unwrap(), "name").unwrap() {
                HkxValue::String { value, .. } => value.clone(),
                _ => panic!(),
            })
            .collect();
        let poses = array(field(skeleton, "referencePose").unwrap()).unwrap();
        let target_nodes = target_worlds(&target).unwrap();
        let mut source_world = Vec::new();
        let mut target_world = Vec::new();
        for (i, pose) in poses.iter().enumerate() {
            let source_parent = if parents[i] < 0 {
                identity()
            } else {
                source_world[parents[i] as usize]
            };
            source_world.push(mat4_mul(&source_parent, &qs_matrix(pose, 1.).unwrap()));
            let target_parent = if parents[i] < 0 {
                identity()
            } else {
                target_world[parents[i] as usize]
            };
            target_world.push(
                map.lookup(&names[i])
                    .and_then(|name| target_nodes.get(&name.to_ascii_lowercase()))
                    .map(|v| v.1)
                    .unwrap_or_else(|| {
                        mat4_mul(&target_parent, &qs_matrix(pose, UNIT_SCALE).unwrap())
                    }),
            );
        }
        let pose = &file.objects()[unique(&file, "hclSimClothPose").unwrap()].members;
        let source_points = point_values(field(pose, "positions").unwrap()).unwrap();
        let skin_id = skin_operator(&file).unwrap();
        let skin_count = skin_vertex_count(&file, &file.objects()[skin_id]).unwrap();
        let vertices = particle_vertices(&file, source_points.len(), skin_count).unwrap();
        let target_skin = proxy_positions(
            &file.objects()[skin_id],
            &target_world,
            skin_count,
            UNIT_SCALE,
        )
        .unwrap();
        let target_points: Vec<_> = vertices.iter().map(|i| target_skin[*i]).collect();
        let scalar = |m: &[HkxMember], name| match field(m, name).unwrap() {
            HkxValue::F32(v) => *v as f64,
            _ => panic!(),
        };
        let bend =
            &file.objects()[unique(&file, "hclBendStiffnessConstraintSetMx").unwrap()].members;
        let max_height_sq = scalar(bend, "maxRestPoseHeightSq");
        let mut largest_cooking_error = 0f64;
        let mut largest_cooking_height_error = 0f64;
        let mut largest_serialized_weight_curvature_error = 0f64;
        let mut largest_repose_motion_change = 0f64;
        let mut largest_authored_rest_residual = 0f64;
        for single in array(field(bend, "singles").unwrap()).unwrap() {
            let m = members(single).unwrap();
            let ids: Vec<_> = ["particleA", "particleB", "particleC", "particleD"]
                .iter()
                .map(|name| index(field(m, name).unwrap()).unwrap())
                .collect();
            let source_p =
                std::array::from_fn(|j| std::array::from_fn(|i| source_points[ids[j]][i]));
            let target_p =
                std::array::from_fn(|j| std::array::from_fn(|i| target_points[ids[j]][i]));
            let weights = ["weightA", "weightB", "weightC", "weightD"].map(|name| scalar(m, name));
            let inv_mass =
                ["invMassA", "invMassB", "invMassC", "invMassD"].map(|name| scalar(m, name));
            let stiffness = scalar(m, "bendStiffness");
            let curvature = scalar(m, "restCurvature");
            let (cooked, cooked_curvature) = cook_bend_primary_f32(&source_p);
            for j in 0..4 {
                assert!((cooked[j] as f64 - weights[j]).abs() < 2e-7);
            }
            let cooking_error = (cooked_curvature as f64 - curvature).abs();
            largest_cooking_error = largest_cooking_error.max(cooking_error);
            let (normal, metric) = bend_shape(&source_p);
            let cooking_height_error = cooking_error * metric;
            largest_cooking_height_error = largest_cooking_height_error.max(cooking_height_error);
            let position_magnitude = source_p.iter().copied().map(bend_length).fold(0., f64::max);
            assert!(
                cooking_height_error < 8. * f32::EPSILON as f64 * position_magnitude,
                "particles {ids:?}: cooking height error {cooking_height_error}"
            );
            let residual: [f64; 3] =
                std::array::from_fn(|i| (0..4).map(|j| source_p[j][i] * weights[j]).sum::<f64>());
            let reconstructed = -(0..3).map(|i| residual[i] * normal[i]).sum::<f64>() / metric;
            largest_serialized_weight_curvature_error =
                largest_serialized_weight_curvature_error.max((reconstructed - curvature).abs());
            assert!((reconstructed - curvature).abs() < 5e-5);
            assert!((-stiffness * inv_mass.iter().sum::<f64>() / 4. - 0.7).abs() < 1e-6);
            let rest = bend_correction(
                &source_p,
                &weights,
                curvature,
                stiffness,
                &inv_mass,
                max_height_sq,
            );
            largest_authored_rest_residual =
                largest_authored_rest_residual.max(bend_length(rest[0]));
            let mut displaced = source_p;
            displaced[0][0] += 0.005;
            displaced[0][1] -= 0.003;
            displaced[0][2] += 0.002;
            let source_result = bend_correction(
                &displaced,
                &weights,
                curvature,
                stiffness,
                &inv_mass,
                max_height_sq,
            );
            let canonical = displaced.map(|p| p.map(|v| v * UNIT_SCALE));
            let canonical_result = bend_correction(
                &canonical,
                &weights,
                curvature / UNIT_SCALE,
                stiffness,
                &inv_mass,
                max_height_sq * UNIT_SCALE.powi(2),
            );
            for j in 0..4 {
                for i in 0..3 {
                    assert!(source_result[j][i].is_finite());
                    assert!(
                        (canonical_result[j][i] / UNIT_SCALE - source_result[j][i]).abs() < 1e-12
                    );
                }
            }
            let (target_weights, target_curvature) = cook_bend(&target_p);
            let target_displaced = std::array::from_fn(|j| {
                std::array::from_fn(|i| {
                    target_p[j][i] + (displaced[j][i] - source_p[j][i]) * UNIT_SCALE
                })
            });
            let target_result = bend_correction(
                &target_displaced,
                &target_weights,
                target_curvature,
                stiffness,
                &inv_mass,
                max_height_sq * UNIT_SCALE.powi(2),
            );
            for j in 0..4 {
                for i in 0..3 {
                    largest_repose_motion_change = largest_repose_motion_change
                        .max((target_result[j][i] / UNIT_SCALE - source_result[j][i]).abs());
                }
            }
        }
        assert!(largest_authored_rest_residual > 1e-4);
        assert!(largest_repose_motion_change > 1e-4);
        let canonical_points: Vec<_> = source_points
            .iter()
            .map(|p| [p[0] * UNIT_SCALE, p[1] * UNIT_SCALE, p[2] * UNIT_SCALE, 1.])
            .collect();
        let mut anchored = file.clone();
        adapt_known_fields(&mut anchored, &source_points, &canonical_points, skin_count).unwrap();
        let op = &mut anchored.objects_mut()[skin_id].members;
        let subset: Vec<_> = array(field(op, "transformSubset").unwrap())
            .unwrap()
            .iter()
            .map(|v| index(v).unwrap())
            .collect();
        let HkxValue::Array(binds) = field_mut(op, "boneFromSkinMeshTransforms").unwrap() else {
            panic!()
        };
        for (bind, bone) in binds.iter_mut().zip(subset) {
            let mut canonical_world = source_world[bone];
            for row in 0..3 {
                canonical_world[row][3] *= UNIT_SCALE;
            }
            let correction = mat4_mul(
                &mat4_affine_inverse(&target_world[bone]).unwrap(),
                &canonical_world,
            );
            *bind = matrix_value(&mat4_mul(&correction, &value_matrix(bind).unwrap()));
        }
        let actual =
            proxy_positions(&anchored.objects()[skin_id], &target_world, skin_count, 1.).unwrap();
        let original =
            proxy_positions(&file.objects()[skin_id], &source_world, skin_count, 1.).unwrap();
        for (a, b) in actual.iter().zip(&original) {
            for i in 0..3 {
                assert!((a[i] - b[i] * UNIT_SCALE).abs() < 2e-5);
            }
        }
        let mut target_animated = Vec::new();
        let mut source_animated = Vec::new();
        for (target, source) in target_world.iter().zip(&source_world) {
            let angle = 0.05 + target[0][3] * 1e-5;
            let mut delta = identity();
            delta[0][0] = angle.cos();
            delta[0][1] = -angle.sin();
            delta[1][0] = angle.sin();
            delta[1][1] = angle.cos();
            delta[0][3] = 0.1 + target[1][3] * 1e-5;
            delta[1][3] = -0.2;
            delta[2][3] = 0.3;
            target_animated.push(mat4_mul(&delta, target));
            for row in 0..3 {
                delta[row][3] /= UNIT_SCALE;
            }
            source_animated.push(mat4_mul(&delta, source));
        }
        let actual_animated = proxy_positions(
            &anchored.objects()[skin_id],
            &target_animated,
            skin_count,
            1.,
        )
        .unwrap();
        let expected_animated =
            proxy_positions(&file.objects()[skin_id], &source_animated, skin_count, 1.).unwrap();
        let animated_error = actual_animated
            .iter()
            .zip(&expected_animated)
            .flat_map(|(a, b)| (0..3).map(move |i| (a[i] - b[i] * UNIT_SCALE).abs()))
            .fold(0., f64::max);
        assert!(animated_error < 2e-5);
        let sim = &file.objects()[unique(&file, "hclSimClothData").unwrap()].members;
        let triangles: Vec<_> = array(field(sim, "triangleIndices").unwrap())
            .unwrap()
            .iter()
            .map(|v| index(v).unwrap())
            .collect();
        let deform =
            &file.objects()[unique(&file, "hclSimpleMeshBoneDeformOperator").unwrap()].members;
        let actual_particles: Vec<_> = vertices.iter().map(|i| actual_animated[*i]).collect();
        let expected_particles: Vec<_> = vertices.iter().map(|i| expected_animated[*i]).collect();
        let mut scaling = identity();
        for i in 0..3 {
            scaling[i][i] = UNIT_SCALE;
        }
        let inverse_scaling = mat4_affine_inverse(&scaling).unwrap();
        let mut transport_error = 0f64;
        for pair in array(field(deform, "triangleBonePairs").unwrap()).unwrap() {
            let m = members(pair).unwrap();
            let bone = index(field(m, "boneOffset").unwrap()).unwrap() / 64;
            let offset = index(field(m, "triangleOffset").unwrap()).unwrap();
            let f_source = triangle_frame(&source_points, &triangles, offset).unwrap();
            let f_source_animated =
                triangle_frame(&expected_particles, &triangles, offset).unwrap();
            let f_canonical = triangle_frame(&canonical_points, &triangles, offset).unwrap();
            let f_actual = triangle_frame(&actual_particles, &triangles, offset).unwrap();
            let output = mat4_mul(
                &f_actual,
                &mat4_mul(
                    &mat4_affine_inverse(&f_canonical).unwrap(),
                    &target_world[bone],
                ),
            );
            let expected = mat4_mul(
                &mat4_mul(
                    &mat4_mul(&scaling, &f_source_animated),
                    &mat4_mul(&mat4_affine_inverse(&f_source).unwrap(), &inverse_scaling),
                ),
                &target_world[bone],
            );
            transport_error = transport_error.max(residual(&output, &expected));
        }
        assert!(transport_error < 1e-4);
        let output = std::env::temp_dir().join("b21_starfield_bend_cooking_proof.json");
        let proof = json!({"source_cook_curvature_error":largest_cooking_error,
            "source_cook_rest_height_error_m":largest_cooking_height_error,
            "serialized_weight_curvature_reconstruction_error":largest_serialized_weight_curvature_error,
            "source_rest_particle_correction_m":largest_authored_rest_residual,
            "reposed_displaced_correction_difference_m":largest_repose_motion_change,
            "canonical_proxy_anchor_error_fo4_units_max":actual.iter().zip(&original).flat_map(|(a,b)|(0..3).map(move |i|(a[i]-b[i]*UNIT_SCALE).abs())).fold(0.,f64::max),
            "animated_proxy_anchor_error_fo4_units_max":animated_error,
            "custom_bone_transport_error_max":transport_error});
        std::fs::write(&output, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
        eprintln!("Starfield bend cooking proof: {}", output.display());
    }

    #[test]
    #[ignore = "requires actual Starfield BendMx source outfits and FO4 skeleton"]
    fn marked_bend_mx_reloads_canonical_proxy_and_authored_coefficients() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let target = NifFile::load(
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        )
        .unwrap();
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let mut receipts = Vec::new();
        for path in [
            "clothes/outfit_akila_security/outfit_akila_security_upperbody_m.nif",
            "clothes/outfit_city/outfit_city_f.nif",
        ] {
            let bytes = std::fs::read(repo.join("extracted/starfield/meshes").join(path)).unwrap();
            let blob = crate::cloth::extract_cloth_blob(&bytes).unwrap();
            assert!(adapt_starfield_cloth(&blob, &target, &map)
                .err()
                .unwrap()
                .contains("hclBendStiffnessConstraintSetMx"));
            let source = hkx::parse_tagfile(&blob)
                .unwrap()
                .materialize_hkx()
                .unwrap();
            let adapted = adapt_starfield_cloth_with_capabilities(
                &blob,
                &target,
                &map,
                &ClothRuntimeCapabilities { bend_mx_v1: true },
            )
            .unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(adapted.runtime_requirements, [BEND_MX_CAPABILITY]);
            let reloaded = HkxFile::read(&adapted.blob).unwrap();
            let original_points = point_values(
                field(
                    &source.objects()[unique(&source, "hclSimClothPose").unwrap()].members,
                    "positions",
                )
                .unwrap(),
            )
            .unwrap();
            let actual_points = point_values(
                field(
                    &reloaded.objects()[unique(&reloaded, "hclSimClothPose").unwrap()].members,
                    "positions",
                )
                .unwrap(),
            )
            .unwrap();
            for (a, b) in original_points.iter().zip(&actual_points) {
                for i in 0..3 {
                    assert!((a[i] * UNIT_SCALE - b[i]).abs() < 1e-5);
                }
            }
            let original = source
                .objects()
                .iter()
                .filter(|o| o.class_name == "hclBendStiffnessConstraintSetMx")
                .collect::<Vec<_>>();
            let lowered = reloaded
                .objects()
                .iter()
                .filter(|o| o.class_name == "hclBendStiffnessConstraintSetMx")
                .collect::<Vec<_>>();
            assert_eq!(original.len(), lowered.len());
            for (before, after) in original.iter().zip(&lowered) {
                let HkxValue::F32(cutoff) = field(&before.members, "maxRestPoseHeightSq").unwrap()
                else {
                    panic!()
                };
                let HkxValue::String { value: name, .. } = field(&after.members, "name").unwrap()
                else {
                    panic!()
                };
                assert!(name.starts_with(&format!(
                    "{BEND_MX_MARKER}{:08X};",
                    ((*cutoff as f64 * UNIT_SCALE.powi(2)) as f32).to_bits()
                )));
                assert!(field(&after.members, "clampBendStiffness").is_err());
                let source_batches = array(field(&before.members, "batches").unwrap()).unwrap();
                let target_batches = array(field(&after.members, "batches").unwrap()).unwrap();
                assert_eq!(source_batches.len(), target_batches.len());
                for (a, b) in source_batches.iter().zip(target_batches) {
                    for member in members(a).unwrap() {
                        let source_values = array(&member.value).unwrap();
                        let target_values =
                            array(field(members(b).unwrap(), &member.name).unwrap()).unwrap();
                        assert_eq!(target_values.len(), 16);
                        for (a, b) in source_values.iter().zip(target_values) {
                            if member.name == "restCurvatures" {
                                let mut expected = a.clone();
                                scaled_number(&mut expected, 1. / UNIT_SCALE).unwrap();
                                assert_eq!(&expected, b);
                            } else {
                                assert_eq!(a, b);
                            }
                        }
                        if member.name == "bendStiffnesses" {
                            assert!(target_values[4..].iter().all(|v| v == &HkxValue::F32(0.)));
                        }
                    }
                }
                let source_singles = array(field(&before.members, "singles").unwrap()).unwrap();
                let target_singles = array(field(&after.members, "singles").unwrap()).unwrap();
                assert_eq!(source_singles.len(), target_singles.len());
                for (a, b) in source_singles.iter().zip(target_singles) {
                    for member in members(a).unwrap() {
                        let mut expected = member.value.clone();
                        if member.name == "restCurvature" {
                            scaled_number(&mut expected, 1. / UNIT_SCALE).unwrap();
                        }
                        assert_eq!(&expected, field(members(b).unwrap(), &member.name).unwrap());
                    }
                }
            }
            receipts.push(json!({"path":path,"runtime_requirements":adapted.runtime_requirements,"bones":adapted.bones.len(),"constraints":lowered.len(),"status":"canonical_after_binary_reload"}));
        }
        std::fs::write(
            std::env::temp_dir().join("b21_starfield_marked_bend_reload.json"),
            serde_json::to_vec_pretty(&receipts).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn packed_vectors_follow_fo4_shared_exponent_unpack() {
        let value = |v: &[i16]| {
            HkxValue::Object(vec![HkxMember {
                name: "values".into(),
                value: HkxValue::Array(v.iter().copied().map(HkxValue::I16).collect()),
            }])
        };
        let (point, step) = packed_vector(&value(&[-1126, -3795, 20009, 12416])).unwrap();
        assert_eq!(step, 1. / 16384.);
        assert_eq!(
            point,
            [-1126. / 16384., -3795. / 16384., 20009. / 16384., 1.]
        );
        assert_eq!(
            packed_vector(&value(&[0, 0, 0, 0])).unwrap(),
            ([0., 0., 0., 1.], 0.)
        );
        assert!(packed_vector(&value(&[1, 0, 0, 0])).is_err());
    }

    #[test]
    fn usage_comparison_rejects_changed_write_bits_without_read_changes() {
        let bitset = |bits| {
            HkxValue::Object(vec![HkxMember {
                name: "storage".into(),
                value: HkxValue::Array(vec![HkxValue::U32(bits)]),
            }])
        };
        let fields = vec![
            HkxMember {
                name: "read".into(),
                value: bitset(4),
            },
            HkxMember {
                name: "readBeforeWrite".into(),
                value: bitset(0),
            },
            HkxMember {
                name: "written".into(),
                value: bitset(32),
            },
        ];
        let original = HkxValue::TypedObject {
            class_name: "hclTransformSetUsageTransformTracker".into(),
            members: fields.clone(),
        };
        let mut reloaded = HkxValue::Object(fields);
        assert!(same_usage_value(&original, &reloaded));
        *field_mut(reloaded.as_object_members_mut().unwrap(), "written").unwrap() = bitset(64);
        assert!(!same_usage_value(&original, &reloaded));
    }

    #[test]
    fn mx_batches_preserve_source_dispatch_with_twelve_finite_inert_lanes() {
        fn member(name: &str, value: HkxValue) -> HkxMember {
            HkxMember {
                name: name.into(),
                value,
            }
        }
        fn real(values: &[f32]) -> HkxValue {
            HkxValue::Array(values.iter().copied().map(HkxValue::F32).collect())
        }
        fn ints(values: &[u16]) -> HkxValue {
            HkxValue::Array(values.iter().copied().map(HkxValue::U16).collect())
        }
        fn solve(points: &mut [[f64; 4]], batches: &[HkxValue], standard: bool, active_only: bool) {
            for batch in batches {
                let m = members(batch).unwrap();
                let a: Vec<_> = array(field(m, "particlesA").unwrap())
                    .unwrap()
                    .iter()
                    .map(|v| index(v).unwrap())
                    .collect();
                let b: Vec<_> = array(field(m, "particlesB").unwrap())
                    .unwrap()
                    .iter()
                    .map(|v| index(v).unwrap())
                    .collect();
                let reals = |name| {
                    array(field(m, name).unwrap())
                        .unwrap()
                        .iter()
                        .map(|v| match v {
                            HkxValue::F32(v) => *v as f64,
                            _ => panic!(),
                        })
                        .collect::<Vec<_>>()
                };
                let rest = reals("restLengths");
                let sa = reals(if standard {
                    "stiffnessesA"
                } else {
                    "stiffnesses"
                });
                let sb = if standard {
                    reals("stiffnessesB")
                } else {
                    vec![0.; a.len()]
                };
                let inv: Vec<_> = a
                    .iter()
                    .zip(&b)
                    .map(|(a, b)| {
                        let sq = (0..3)
                            .map(|j| (points[*a][j] - points[*b][j]).powi(2))
                            .sum::<f64>();
                        if sq > 0. {
                            1. / sq.sqrt()
                        } else {
                            0.
                        }
                    })
                    .collect();
                let lanes = if active_only { 4 } else { a.len() };
                for group in (0..lanes).step_by(4) {
                    let previous = points.to_vec();
                    for i in group..group + 4 {
                        let delta: [f64; 3] = std::array::from_fn(|j| {
                            (previous[b[i]][j] - previous[a[i]][j]) * inv[i]
                        });
                        if standard {
                            let error = 1. / inv[i] - rest[i];
                            for j in 0..3 {
                                points[a[i]][j] = previous[a[i]][j] + sa[i] * error * delta[j];
                                points[b[i]][j] = previous[b[i]][j] + sb[i] * error * delta[j];
                            }
                        } else {
                            let error = (rest[i] - 1. / inv[i]).min(0.);
                            for j in 0..3 {
                                points[b[i]][j] = previous[b[i]][j] + sa[i] * error * delta[j];
                            }
                        }
                    }
                }
            }
        }
        let source: Vec<_> = (0..8).map(|i| [i as f64, 0., 0., 1.]).collect();
        let target: Vec<_> = source.iter().map(|p| [p[0] * 2., 0., 0., 1.]).collect();
        for standard in [true, false] {
            let class = if standard {
                "hclStandardLinkConstraintSetMx"
            } else {
                "hclStretchLinkConstraintSetMx"
            };
            let mut fields = vec![
                member("restLengths", real(&[0.7, 0.8, 0.9, 0.6])),
                member("particlesA", ints(&[0, 2, 4, 6])),
                member("particlesB", ints(&[1, 3, 5, 7])),
            ];
            if standard {
                fields.extend([
                    member("stiffnessesA", real(&[0.5; 4])),
                    member("stiffnessesB", real(&[-0.5; 4])),
                ]);
            } else {
                fields.push(member("stiffnesses", real(&[0.5; 4])));
            }
            let batch = HkxValue::TypedObject {
                class_name: format!("{class}Batch"),
                members: fields,
            };
            let mut object = HkxObject {
                name: None,
                offset: 0,
                signature: 0,
                class_name: class.into(),
                members: vec![
                    member("batches", HkxValue::Array(vec![batch.clone(), batch])),
                    member("singles", HkxValue::Array(vec![])),
                ],
            };
            rebase_links(&mut object, &source, &target).unwrap();
            let file = HkxFile::from_tagxml(11, "hk_2014.1.0-r1", vec![object]);
            let bytes = hkx::write_hkx(
                &file,
                &mut DescriptorRegistry::for_contents_version("hk_2014.1.0-r1"),
            );
            let reloaded = HkxFile::read(&bytes).unwrap();
            let batches = array(field(&reloaded.objects()[0].members, "batches").unwrap()).unwrap();
            assert_eq!(batches.len(), 2);
            for batch in batches {
                let m = members(batch).unwrap();
                for f in m {
                    assert_eq!(array(&f.value).unwrap().len(), 16);
                }
                for name in if standard {
                    vec!["stiffnessesA", "stiffnessesB"]
                } else {
                    vec!["stiffnesses"]
                } {
                    for value in &array(field(m, name).unwrap()).unwrap()[4..] {
                        assert_eq!(value, &HkxValue::F32(0.));
                    }
                }
                for (i, expected) in [1.4, 1.6, 1.8, 1.2].iter().enumerate() {
                    let HkxValue::F32(actual) = array(field(m, "restLengths").unwrap()).unwrap()[i]
                    else {
                        panic!()
                    };
                    assert!((actual as f64 - expected).abs() < 1e-6);
                }
            }
            let mut padded = target.clone();
            let mut active = target.clone();
            solve(&mut padded, batches, standard, false);
            solve(&mut active, batches, standard, true);
            assert_eq!(padded, active);
            assert_ne!(padded, target);
            assert!(padded.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    #[ignore = "requires extracted Starfield and Fallout4 skeletons"]
    fn constellation_cloth_adapter_reloads_with_target_rest_and_units() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let nif=repo.join("extracted/starfield/meshes/Clothes/Spacesuit_Constellation/Spacesuit_Constellation_Legs_M.nif");
        let blob = crate::cloth::extract_cloth_blob(&std::fs::read(&nif).unwrap()).unwrap();
        let target = NifFile::load(
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        )
        .unwrap();
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let adapted = adapt_starfield_cloth(&blob, &target, &map).unwrap();
        let reloaded = HkxFile::read(&adapted.blob).unwrap();
        let skel = &reloaded.objects()[unique(&reloaded, "hkaSkeleton").unwrap()].members;
        assert_eq!(array(field(skel, "bones").unwrap()).unwrap().len(), 122);
        let custom: Vec<_> = adapted
            .bones
            .iter()
            .filter(|b| b.custom.is_some())
            .collect();
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].source_name, "SS_L_Cloth_01");
        assert_eq!(custom[0].custom.as_ref().unwrap().parent, "Pelvis");
        assert_eq!(adapted.unbound_unused.len(), 19);
        let parent = target_worlds(&target).unwrap()["pelvis"].1;
        assert!(
            residual(
                &mat4_mul(&parent, &custom[0].custom.as_ref().unwrap().local),
                &custom[0].world
            ) < 1e-6
        );
        let sim = &reloaded.objects()[unique(&reloaded, "hclSimClothData").unwrap()].members;
        let gravity = floats(
            field(
                members(field(sim, "simulationInfo").unwrap()).unwrap(),
                "gravity",
            )
            .unwrap(),
        )
        .unwrap();
        assert!((gravity[2] - (-9.8100004196167 * UNIT_SCALE)).abs() < 1e-4);
        let particles = array(field(sim, "particleDatas").unwrap()).unwrap();
        let HkxValue::F32(radius) = field(members(&particles[0]).unwrap(), "radius").unwrap()
        else {
            panic!()
        };
        assert!((*radius as f64 - 0.009999999776482582 * UNIT_SCALE).abs() < 1e-6);
        let deform = &reloaded.objects()
            [unique(&reloaded, "hclSimpleMeshBoneDeformOperator").unwrap()]
        .members;
        let pair =
            members(&array(field(deform, "triangleBonePairs").unwrap()).unwrap()[0]).unwrap();
        assert_eq!(index(field(pair, "boneOffset").unwrap()).unwrap(), 2048);
        let pose = &reloaded.objects()[unique(&reloaded, "hclSimClothPose").unwrap()].members;
        let points = point_values(field(pose, "positions").unwrap()).unwrap();
        let triangles = array(field(sim, "triangleIndices").unwrap())
            .unwrap()
            .iter()
            .map(index)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let local = value_matrix(&array(field(deform, "localBoneTransforms").unwrap()).unwrap()[0])
            .unwrap();
        let error = residual(
            &mat4_mul(&triangle_frame(&points, &triangles, 0).unwrap(), &local),
            &custom[0].world,
        );
        assert!(error < 1e-3);
        let output = std::env::temp_dir().join("b21_starfield_cloth_adapter");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("constellation_fo4.hkx"), &adapted.blob).unwrap();
        let receipt = json!({"status":"adapted_after_binary_reload","source":nif,"bones":122,"bindings":adapted.bones.len(),
            "custom_bones":custom.iter().map(|b|&b.target_name).collect::<Vec<_>>(),"unbound_unused":adapted.unbound_unused,
            "unit_scale":UNIT_SCALE,"gravity":gravity,"particle_radius":radius,"deform_rest_max_residual":error});
        std::fs::write(
            output.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        eprintln!(
            "Starfield cloth adapter: {}",
            output.join("receipt.json").display()
        );
        let mut missing = target.clone();
        let pelvis = missing
            .blocks
            .iter_mut()
            .find(|b| matches!(b.get_field("Name"),Some(NifValue::String(n))if n=="Pelvis"))
            .unwrap();
        pelvis.set_field("Name", NifValue::String("MissingPelvis".into()));
        assert!(adapt_starfield_cloth(&blob, &missing, &map)
            .err()
            .unwrap()
            .contains("missing Pelvis"));
    }

    #[test]
    #[ignore = "requires extracted Starfield and Fallout4 skeletons"]
    fn ryujin_two_blend_skin_and_local_range_reloads_without_lost_constraints() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let nif = repo.join("extracted/starfield/meshes/clothes/outfit_ryujin_businesssuit_manager_01/outfit_ryujin_businesssuit_manager_sleeves_01_f_3rd.nif");
        let blob = crate::cloth::extract_cloth_blob(&std::fs::read(&nif).unwrap()).unwrap();
        let original = hkx::parse_tagfile(&blob)
            .unwrap()
            .materialize_hkx()
            .unwrap();
        let target = NifFile::load(
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        )
        .unwrap();
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let adapted = adapt_starfield_cloth(&blob, &target, &map).unwrap();
        let reloaded = HkxFile::read(&adapted.blob).unwrap();
        let old =
            &original.objects()[unique(&original, "hclLocalRangeConstraintSet").unwrap()].members;
        let new =
            &reloaded.objects()[unique(&reloaded, "hclLocalRangeConstraintSet").unwrap()].members;
        let old = array(field(old, "localConstraints").unwrap()).unwrap();
        let new = array(field(new, "localConstraints").unwrap()).unwrap();
        assert_eq!(old.len(), new.len());
        assert_eq!(old.len(), 2);
        for (old, new) in old.iter().zip(new) {
            let old = members(old).unwrap();
            let new = members(new).unwrap();
            for name in ["particleIndex", "referenceVertex"] {
                assert_eq!(field(old, name).unwrap(), field(new, name).unwrap());
            }
            for name in ["maximumDistance", "maxNormalDistance", "minNormalDistance"] {
                let (HkxValue::F32(a), HkxValue::F32(b)) =
                    (field(old, name).unwrap(), field(new, name).unwrap())
                else {
                    panic!()
                };
                assert!((*b as f64 - *a as f64 * UNIT_SCALE).abs() < 1e-6);
            }
        }
    }

    #[test]
    #[ignore = "requires extracted Starfield and Fallout4 skeletons"]
    fn delgado_object_skin_and_gather_reloads_preserving_indices() {
        fn same_data(a: &HkxValue, b: &HkxValue) {
            if let (Some(a), Some(b)) = (a.as_object_members(), b.as_object_members()) {
                assert_eq!(a.len(), b.len());
                for member in a {
                    same_data(&member.value, field(b, &member.name).unwrap());
                }
            } else if let (HkxValue::Array(a), HkxValue::Array(b)) = (a, b) {
                assert_eq!(a.len(), b.len());
                for (a, b) in a.iter().zip(b) {
                    same_data(a, b);
                }
            } else {
                assert_eq!(a, b);
            }
        }
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = repo.join(
            "extracted/starfield/meshes/clothes/outfit_delgado/outfit_delgado_lowerbody_m.nif",
        );
        let blob = crate::cloth::extract_cloth_blob(&std::fs::read(&source).unwrap()).unwrap();
        let original = hkx::parse_tagfile(&blob)
            .unwrap()
            .materialize_hkx()
            .unwrap();
        let target = NifFile::load(
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        )
        .unwrap();
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let adapted = adapt_starfield_cloth(&blob, &target, &map).unwrap();
        let reloaded = HkxFile::read(&adapted.blob).unwrap();
        let old = &original.objects()[skin_operator(&original).unwrap()].members;
        let new = &reloaded.objects()[skin_operator(&reloaded).unwrap()].members;
        assert!(!array(field(old, "localPNs").unwrap()).unwrap().is_empty());
        assert!(array(field(new, "localPNs").unwrap()).unwrap().is_empty());
        assert_eq!(
            array(field(old, "localPNs").unwrap()).unwrap().len(),
            array(field(new, "localUnpackedPNs").unwrap())
                .unwrap()
                .len()
        );
        for name in ["transformSubset", "outputBufferIndex", "transformSetIndex"] {
            assert_eq!(field(old, name).unwrap(), field(new, name).unwrap());
        }
        let old_d = members(field(old, "objectSpaceDeformer").unwrap()).unwrap();
        let new_d = members(field(new, "objectSpaceDeformer").unwrap()).unwrap();
        for name in [
            "fourBlendEntries",
            "threeBlendEntries",
            "twoBlendEntries",
            "oneBlendEntries",
            "controlBytes",
            "startVertexIndex",
            "endVertexIndex",
            "partialWrite",
        ] {
            same_data(field(old_d, name).unwrap(), field(new_d, name).unwrap());
        }
        let old =
            &original.objects()[unique(&original, "hclGatherAllVerticesOperator").unwrap()].members;
        let new =
            &reloaded.objects()[unique(&reloaded, "hclGatherAllVerticesOperator").unwrap()].members;
        assert_eq!(
            field(old, "vertexInputFromVertexOutput").unwrap(),
            field(new, "vertexInputFromVertexOutput").unwrap()
        );
        let report: Value =
            serde_json::from_str(&api::cloth_validate(&adapted.blob).unwrap()).unwrap();
        assert_eq!(report["valid"], true, "{report}");
    }

    #[test]
    #[ignore = "requires extracted Starfield and Fallout4 skeletons"]
    fn masako_capsules_repose_to_target_bones_with_inverse_length_cache() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = repo
            .join("extracted/starfield/meshes/clothes/outfit_masako/outfit_masako_upperbody_f.nif");
        let blob = crate::cloth::extract_cloth_blob(&std::fs::read(&source).unwrap()).unwrap();
        let original = hkx::parse_tagfile(&blob)
            .unwrap()
            .materialize_hkx()
            .unwrap();
        let target = NifFile::load(
            repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
        )
        .unwrap();
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let adapted = adapt_starfield_cloth(&blob, &target, &map).unwrap();
        let reloaded = HkxFile::read(&adapted.blob).unwrap();
        let target_bones = target_worlds(&target).unwrap();
        let sim = &reloaded.objects()[unique(&reloaded, "hclSimClothData").unwrap()].members;
        let mapping = members(field(sim, "collidableTransformMap").unwrap()).unwrap();
        let indices = array(field(mapping, "transformIndices").unwrap()).unwrap();
        let offsets = array(field(mapping, "offsets").unwrap()).unwrap();
        let skel = &reloaded.objects()[unique(&reloaded, "hkaSkeleton").unwrap()].members;
        let bones = array(field(skel, "bones").unwrap()).unwrap();
        let colliders = array(field(sim, "perInstanceCollidables").unwrap()).unwrap();
        assert_eq!(colliders.len(), 2);
        for ((collider, bone), offset) in colliders.iter().zip(indices).zip(offsets) {
            let bone = members(&bones[index(bone).unwrap()]).unwrap();
            let HkxValue::String { value: name, .. } = field(bone, "name").unwrap() else {
                panic!()
            };
            let expected = mat4_mul(
                &target_bones[&name.to_ascii_lowercase()].1,
                &transform_matrix(offset).unwrap(),
            );
            let HkxValue::Pointer(Some(id)) = collider else {
                panic!()
            };
            let actual =
                transform_matrix(field(&reloaded.objects()[*id].members, "transform").unwrap())
                    .unwrap();
            assert!(residual(&expected, &actual) < 1e-4);
        }
        let old_shapes: Vec<_> = original
            .objects()
            .iter()
            .filter(|o| o.class_name == "hclCapsuleShape")
            .collect();
        let new_shapes: Vec<_> = reloaded
            .objects()
            .iter()
            .filter(|o| o.class_name == "hclCapsuleShape")
            .collect();
        assert_eq!(new_shapes.len(), old_shapes.len());
        assert_eq!(new_shapes.len(), 2);
        for (old, new) in old_shapes.iter().zip(new_shapes) {
            for name in ["start", "end"] {
                let a = floats(field(&old.members, name).unwrap()).unwrap();
                let b = floats(field(&new.members, name).unwrap()).unwrap();
                assert!((0..3).all(|j| (a[j] * UNIT_SCALE - b[j]).abs() < 1e-5));
            }
            let scalar = |o: &HkxObject, name| match field(&o.members, name).unwrap() {
                HkxValue::F32(v) => *v as f64,
                _ => panic!(),
            };
            assert!((scalar(new, "radius") - scalar(old, "radius") * UNIT_SCALE).abs() < 1e-5);
            let start = floats(field(&new.members, "start").unwrap()).unwrap();
            let end = floats(field(&new.members, "end").unwrap()).unwrap();
            let length_sq = (0..3).map(|j| (end[j] - start[j]).powi(2)).sum::<f64>();
            assert!((scalar(new, "capLenSqrdInv") * length_sq - 1.).abs() < 1e-5);
        }
        let report: Value =
            serde_json::from_str(&api::cloth_validate(&adapted.blob).unwrap()).unwrap();
        assert_eq!(report["valid"], true, "{report}");
        std::fs::write(
            std::env::temp_dir().join("b21_starfield_cloth_adapter/masako_fo4.hkx"),
            adapted.blob,
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires complete extracted human Starfield ARMA manifest and Fallout4 rigs"]
    fn human_cloth_cohort_preserves_authored_simulation_contracts() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(std::env::temp_dir().join("b21_starfield_armor_family_manifest.json"))
                .unwrap(),
        )
        .unwrap();
        let mut jobs = BTreeSet::new();
        for record in manifest["records"].as_array().unwrap() {
            if record["signature"] != "ARMA"
                || !record["races"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["object_id"] == "00347D")
            {
                continue;
            }
            for (role, paths) in record["models"].as_object().unwrap() {
                if !matches!(role.as_str(), "MOD2" | "MOD3" | "MOD4" | "MOD5") {
                    continue;
                }
                for path in paths.as_array().unwrap() {
                    jobs.insert((
                        path.as_str()
                            .unwrap()
                            .replace('\\', "/")
                            .to_ascii_lowercase(),
                        matches!(role.as_str(), "MOD4" | "MOD5"),
                    ));
                }
            }
        }
        assert_eq!(jobs.len(), 1419);
        let rigs = [
            NifFile::load(
                repo.join("extracted/fo4/meshes/actors/character/characterassets/skeleton.nif"),
            )
            .unwrap(),
            NifFile::load(repo.join(
                "extracted/fo4/meshes/actors/character/_1stperson/characterassets/skeleton.nif",
            ))
            .unwrap(),
        ];
        let map = SkeletonMap::load(
            &repo.join("bacup/py_bacup_lib/native/conversion/src/embedded/translation_maps"),
            "starfield",
            "fo4",
        )
        .unwrap();
        let output = std::env::temp_dir().join("b21_starfield_cloth_cohort");
        std::fs::create_dir_all(&output).unwrap();
        let mut results = Vec::new();
        let mut passed = 0;
        let mut failed = 0;
        let mut missing = Vec::new();
        for (path, first) in jobs {
            let source = repo.join("extracted/starfield/meshes").join(&path);
            if !source.exists() {
                missing.push(json!({"path":path,"perspective":if first{"1P"}else{"3P"}}));
                continue;
            }
            let data = std::fs::read(&source).unwrap();
            let nif = NifFile::from_bytes(&data, None).unwrap();
            if !nif.blocks.iter().any(|b| b.type_name == "BSClothExtraData") {
                continue;
            }
            for (i, blob) in crate::cloth::extract_cloth_blobs(&data)
                .unwrap()
                .iter()
                .enumerate()
            {
                let original = hkx::parse_tagfile(blob).unwrap().materialize_hkx().unwrap();
                let classes: BTreeSet<_> = original
                    .objects()
                    .iter()
                    .map(|o| o.class_name.clone())
                    .collect();
                let result = adapt_starfield_cloth(blob, &rigs[usize::from(first)], &map).and_then(
                    |adapted| {
                        let report: Value = serde_json::from_str(
                            &api::cloth_validate(&adapted.blob).map_err(|e| e.to_string())?,
                        )
                        .map_err(|e| e.to_string())?;
                        if report["valid"] != true {
                            return Err(format!("target cloth validation failed: {report}"));
                        }
                        Ok(adapted)
                    },
                );
                let mut row = match result {
                    Ok(adapted) => {
                        passed += 1;
                        std::fs::write(output.join(format!("{passed:04}.hkx")), &adapted.blob)
                            .unwrap();
                        json!({"path":path,"perspective":if first{"1P"}else{"3P"},"blob":i,"status":"passed","bindings":adapted.bones.len(),"custom_bones":adapted.bones.iter().filter(|b|b.custom.is_some()).map(|b|&b.source_name).collect::<Vec<_>>(),"unbound_unused":adapted.unbound_unused})
                    }
                    Err(error) => {
                        failed += 1;
                        json!({"path":path,"perspective":if first{"1P"}else{"3P"},"blob":i,"status":"failed","error":error})
                    }
                };
                row["source_classes"] = json!(classes);
                results.push(row);
            }
        }
        let receipt = json!({"passed":passed,"failed":failed,"cloth_blobs":results.len(),"missing_jobs":missing,"results":results});
        std::fs::write(
            output.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        eprintln!(
            "Starfield cloth cohort: {passed} passed/{failed} failed; {}",
            output.join("receipt.json").display()
        );
        assert_eq!(missing.len(), 2);
        assert_eq!(failed, 0, "unsupported human cloth remains; see receipt");
    }
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn graph_snapshot(graph: &Value) -> BTreeMap<String, Value> {
        let mut counts = BTreeMap::<String, usize>::new();
        let mut snapshot = BTreeMap::new();
        for object in graph["objects"].as_array().unwrap() {
            let class = object["class"].as_str().unwrap();
            let fields: &[&str] = match class {
                "hkaSkeleton" => &["bones", "parentIndices", "referencePose"],
                "hclTransformSetDefinition" => &["numTransforms"],
                "hclBoneSpaceSkinPNOperator" => &[
                    "transformSubset",
                    "transformSetIndex",
                    "outputBufferIndex",
                    "localPNs",
                    "localUnpackedPNs",
                ],
                "hclSimpleMeshBoneDeformOperator" => &[
                    "triangleBonePairs",
                    "localBoneTransforms",
                    "outputTransformSetIdx",
                ],
                "hclBonePlanesConstraintSet" => &["bonePlanes", "transformSetIndex"],
                "hclSimClothPose" => &["positions"],
                "hclStandardLinkConstraintSetMx" | "hclStretchLinkConstraintSetMx" => {
                    &["singles", "batches"]
                }
                "hclSimClothData" => &[
                    "particleDatas",
                    "fixedParticles",
                    "triangleIndices",
                    "triangleFlips",
                    "collidableTransformMap",
                    "transferMotionData",
                ],
                "hclClothState" => &["operators", "usedSimCloths"],
                "hclMoveParticlesOperator" => {
                    &["vertexParticlePairs", "simClothIndex", "refBufferIdx"]
                }
                "hclCopyVerticesOperator" => &[
                    "inputBufferIdx",
                    "outputBufferIdx",
                    "numberOfVertices",
                    "startVertexIn",
                    "startVertexOut",
                    "copyNormals",
                ],
                _ if class.starts_with("hcl") && class.ends_with("Operator") => &[],
                _ => continue,
            };
            let ordinal = counts.entry(class.into()).or_default();
            let mut selected: serde_json::Map<String, Value> = fields
                .iter()
                .map(|field| ((*field).into(), object["members"][*field].clone()))
                .collect();
            if class == "hclClothState" {
                for field in ["usedBuffers", "usedTransformSets"] {
                    selected.insert(field.into(), object["members"][field].clone());
                }
            }
            if let Some(blocks) = selected.get_mut("localPNs").and_then(Value::as_array_mut) {
                for block in blocks {
                    let Some(normals) = block.get_mut("localNormal") else {
                        continue;
                    };
                    let Some(packed) = normals.as_array().filter(|values| {
                        !values.is_empty() && values.iter().all(|value| value["values"].is_array())
                    }) else {
                        continue;
                    };
                    *normals = Value::Array(
                        packed
                            .iter()
                            .flat_map(|value| value["values"].as_array().unwrap().clone())
                            .collect(),
                    );
                }
            }
            snapshot.insert(format!("{class}:{ordinal}"), Value::Object(selected));
            *ordinal += 1;
        }
        snapshot
    }

    #[test]
    #[ignore = "requires extracted Starfield Constellation cloth; probes schema lowering, not runtime adaptation"]
    fn constellation_cloth_schema_lowering_probe() {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let nif = repo.join("extracted/starfield/meshes/Clothes/Spacesuit_Constellation/Spacesuit_Constellation_Legs_M.nif");
        let blob = crate::cloth::extract_cloth_blob(&std::fs::read(&nif).unwrap()).unwrap();
        let tagfile = hkx::parse_tagfile(&blob).unwrap();
        assert_eq!(tagfile.sdk_version, "20190200");
        let generic = api::havok_convert_bytes_report(&blob, "fo4");
        let generic_error = generic.as_ref().err().map(ToString::to_string);
        let source: Value =
            serde_json::from_str(&api::cloth_inspect_blob_json(&blob).unwrap()).unwrap();
        let source_validation: Value =
            serde_json::from_str(&api::cloth_validate(&blob).unwrap()).unwrap();

        let lowered = migrate_2015_packfile_to_2014_with_warnings(
            tagfile.materialize_hkx().unwrap(),
            Fo76MigrationOptions::default(),
        )
        .unwrap();
        let mut registry = DescriptorRegistry::for_contents_version("hk_2014.1.0-r1");
        let target_blob = hkx::write_hkx(&lowered.hkx, &mut registry);
        let target: Value =
            serde_json::from_str(&api::cloth_inspect_blob_json(&target_blob).unwrap()).unwrap();
        let target_validation: Value =
            serde_json::from_str(&api::cloth_validate(&target_blob).unwrap()).unwrap();
        let format = api::hkx_detect_format_full(&target_blob).unwrap();
        let source_snapshot = graph_snapshot(&source);
        let target_snapshot = graph_snapshot(&target);
        let output = std::env::temp_dir().join("b21_starfield_cloth_schema_probe");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("constellation_fo4_unadapted.hkx"), &target_blob).unwrap();
        let receipt = json!({
            "purpose":"schema lowering probe; units, target bone binding and repose are not applied",
            "source":nif, "source_sdk":tagfile.sdk_version,
            "generic_conversion_error":generic_error,
            "target_format":format.kind, "target_version":format.version,
            "lowering_warnings":lowered.warnings,
            "preserved_snapshot":source_snapshot == target_snapshot,
            "source_snapshot":source_snapshot, "target_snapshot":target_snapshot,
            "source_validation":source_validation, "target_validation":target_validation,
        });
        let receipt_path = output.join("receipt.json");
        std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        eprintln!("Starfield cloth schema probe: {}", receipt_path.display());
        assert!(
            generic.is_err(),
            "generic 2019 conversion is now available; update this probe"
        );
        assert_eq!(format.kind, "packfile");
        assert_eq!(format.version, "hk_2014.1.0-r1");
        assert!(
            source_snapshot == target_snapshot,
            "schema lowering lost authored cloth structure; see {}",
            receipt_path.display()
        );
    }
}
