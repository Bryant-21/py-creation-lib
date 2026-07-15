use indexmap::IndexMap;

use crate::model::NifValue;
use crate::skin::bone_remap::VertexInfluences;

const VF_VERTEX: i64 = 0x0001;
const VF_UVS: i64 = 0x0002;
const VF_NORMALS: i64 = 0x0008;
const VF_TANGENTS: i64 = 0x0010;
const VF_VERTEX_COLORS: i64 = 0x0020;
const VF_SKINNED: i64 = 0x0040;
const VF_FULLPRECISION: i64 = 0x4000;

pub fn vertex_desc_skinned(has_vertex_colors: bool) -> i64 {
    let mut stride = 7i64;
    let mut flags = VF_VERTEX | VF_UVS | VF_NORMALS | VF_TANGENTS | VF_SKINNED | VF_FULLPRECISION;
    let mut color_offset = 0i64;
    if has_vertex_colors {
        stride += 1;
        flags |= VF_VERTEX_COLORS;
        color_offset = 7;
    }
    let normal_offset = 3i64;
    let tangent_offset = 4i64;
    let skin_offset = 5i64;
    stride
        | (normal_offset << 8)
        | (tangent_offset << 16)
        | (skin_offset << 20)
        | (color_offset << 24)
        | (flags << 44)
}

pub fn vertex_desc_static(has_vertex_colors: bool) -> i64 {
    let stride = if has_vertex_colors { 6 } else { 5 };
    let mut flags = VF_VERTEX | VF_UVS | VF_NORMALS | VF_TANGENTS;
    let mut color_offset = 0;
    if has_vertex_colors {
        flags |= VF_VERTEX_COLORS;
        color_offset = 5;
    }
    stride | (2 << 8) | (3 << 16) | (4 << 20) | (color_offset << 24) | (flags << 44)
}

pub fn pack_static_vertex_data(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    tangents: &[[f32; 3]],
    bitangents: &[[f32; 3]],
    uvs: &[[f32; 2]],
    vertex_colors: Option<&[[f32; 4]]>,
) -> Vec<NifValue> {
    let mut out = Vec::with_capacity(positions.len());
    for i in 0..positions.len() {
        let mut entry = IndexMap::new();
        let pos = positions.get(i).copied().unwrap_or([0.0; 3]);
        let bitan = bitangents.get(i).copied().unwrap_or([0.0; 3]);
        let normal = normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
        let tangent = tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0]);
        let uv = uvs.get(i).copied().unwrap_or([0.0, 0.0]);

        entry.insert("Vertex".into(), NifValue::Vec3(pos));
        entry.insert("Bitangent X".into(), NifValue::Float(bitan[0] as f64));
        entry.insert(
            "UV".into(),
            NifValue::Struct({
                let mut tex = IndexMap::new();
                tex.insert("u".into(), NifValue::Float(uv[0] as f64));
                tex.insert("v".into(), NifValue::Float(uv[1] as f64));
                tex
            }),
        );
        entry.insert("Normal".into(), NifValue::Vec3(normal));
        entry.insert("Bitangent Y".into(), NifValue::Float(bitan[1] as f64));
        entry.insert("Tangent".into(), NifValue::Vec3(tangent));
        entry.insert("Bitangent Z".into(), NifValue::Float(bitan[2] as f64));

        if let Some(colors) = vertex_colors {
            let color = colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
            entry.insert("Vertex Colors".into(), NifValue::Color4(color));
        }

        out.push(NifValue::Struct(entry));
    }
    out
}

pub fn pack_skinned_vertex_data(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    tangents: &[[f32; 3]],
    bitangents: &[[f32; 3]],
    uvs: &[[f32; 2]],
    vertex_colors: Option<&[[f32; 4]]>,
    influences: &[VertexInfluences],
) -> Vec<NifValue> {
    let mut out = Vec::with_capacity(positions.len());
    for i in 0..positions.len() {
        let mut entry = IndexMap::new();
        let pos = positions.get(i).copied().unwrap_or([0.0; 3]);
        let bitan = bitangents.get(i).copied().unwrap_or([0.0; 3]);
        let normal = normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
        let tangent = tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0]);
        let uv = uvs.get(i).copied().unwrap_or([0.0, 0.0]);

        entry.insert("Vertex".into(), NifValue::Vec3(pos));
        entry.insert("Bitangent X".into(), NifValue::Float(bitan[0] as f64));
        entry.insert(
            "UV".into(),
            NifValue::Struct({
                let mut tex = IndexMap::new();
                tex.insert("u".into(), NifValue::Float(uv[0] as f64));
                tex.insert("v".into(), NifValue::Float(uv[1] as f64));
                tex
            }),
        );
        entry.insert("Normal".into(), NifValue::Vec3(normal));
        entry.insert("Bitangent Y".into(), NifValue::Float(bitan[1] as f64));
        entry.insert("Tangent".into(), NifValue::Vec3(tangent));
        entry.insert("Bitangent Z".into(), NifValue::Float(bitan[2] as f64));

        if let Some(colors) = vertex_colors {
            let color = colors.get(i).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
            entry.insert("Vertex Colors".into(), NifValue::Color4(color));
        }

        let mut indices = [0u8; 4];
        let mut weights = [0.0_f32; 4];
        if let Some(inf) = influences.get(i) {
            for (slot, (bone_index, bone_weight)) in inf.slots.iter().enumerate().take(4) {
                indices[slot] = (*bone_index).min(255) as u8;
                weights[slot] = *bone_weight;
            }
        }
        entry.insert(
            "Bone Indices".into(),
            NifValue::Array(
                indices
                    .iter()
                    .map(|value| NifValue::UInt(*value as u64))
                    .collect(),
            ),
        );
        entry.insert(
            "Bone Weights".into(),
            NifValue::Array(
                weights
                    .iter()
                    .map(|value| NifValue::Float(*value as f64))
                    .collect(),
            ),
        );

        out.push(NifValue::Struct(entry));
    }
    out
}

pub fn recompute_tangents_lengyel(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    triangles: &[[u32; 3]],
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let vertex_count = positions.len();
    let mut tangent_accum = vec![[0.0_f32; 3]; vertex_count];
    let mut bitangent_accum = vec![[0.0_f32; 3]; vertex_count];

    for triangle in triangles {
        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;
        if i0 >= vertex_count || i1 >= vertex_count || i2 >= vertex_count {
            continue;
        }

        let v0 = positions[i0];
        let v1 = positions[i1];
        let v2 = positions[i2];
        let uv0 = uvs.get(i0).copied().unwrap_or([0.0, 0.0]);
        let uv1 = uvs.get(i1).copied().unwrap_or([0.0, 0.0]);
        let uv2 = uvs.get(i2).copied().unwrap_or([0.0, 0.0]);

        let edge1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let edge2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let delta_uv1 = [uv1[0] - uv0[0], uv1[1] - uv0[1]];
        let delta_uv2 = [uv2[0] - uv0[0], uv2[1] - uv0[1]];

        let denom = delta_uv1[0] * delta_uv2[1] - delta_uv2[0] * delta_uv1[1];
        if denom.abs() <= 1e-12 {
            continue;
        }
        let reciprocal = 1.0 / denom;

        let sdir = [
            (delta_uv2[1] * edge1[0] - delta_uv1[1] * edge2[0]) * reciprocal,
            (delta_uv2[1] * edge1[1] - delta_uv1[1] * edge2[1]) * reciprocal,
            (delta_uv2[1] * edge1[2] - delta_uv1[1] * edge2[2]) * reciprocal,
        ];
        let tdir = [
            (delta_uv1[0] * edge2[0] - delta_uv2[0] * edge1[0]) * reciprocal,
            (delta_uv1[0] * edge2[1] - delta_uv2[0] * edge1[1]) * reciprocal,
            (delta_uv1[0] * edge2[2] - delta_uv2[0] * edge1[2]) * reciprocal,
        ];

        for vertex_index in [i0, i1, i2] {
            add_assign(&mut tangent_accum[vertex_index], sdir);
            add_assign(&mut bitangent_accum[vertex_index], tdir);
        }
    }

    let mut tangents = vec![[1.0_f32, 0.0, 0.0]; vertex_count];
    let mut bitangents = vec![[0.0_f32, 1.0, 0.0]; vertex_count];
    for index in 0..vertex_count {
        let normal = normals.get(index).copied().unwrap_or([0.0, 0.0, 1.0]);
        let tangent = orthonormalize_tangent(normal, tangent_accum[index]);
        tangents[index] = tangent;

        let mut bitangent = cross(normal, tangent);
        if dot(bitangent, bitangent_accum[index]) < 0.0 {
            bitangent = [-bitangent[0], -bitangent[1], -bitangent[2]];
        }
        bitangents[index] = normalize_or(bitangent, [0.0, 1.0, 0.0]);
    }

    (tangents, bitangents)
}

fn add_assign(dst: &mut [f32; 3], src: [f32; 3]) {
    dst[0] += src[0];
    dst[1] += src[1];
    dst[2] += src[2];
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}

fn normalize_or(vector: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let length = dot(vector, vector).sqrt();
    if length <= 1e-8 {
        return fallback;
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

fn orthonormalize_tangent(normal: [f32; 3], tangent: [f32; 3]) -> [f32; 3] {
    let projection = dot(normal, tangent);
    let tangent = [
        tangent[0] - normal[0] * projection,
        tangent[1] - normal[1] * projection,
        tangent[2] - normal[2] * projection,
    ];
    normalize_or(tangent, [1.0, 0.0, 0.0])
}
