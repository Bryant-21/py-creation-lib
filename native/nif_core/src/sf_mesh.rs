//! Starfield `.mesh` binary reader — external geometry for `BSGeometry` blocks.
//!
//! Layout ported 1:1 from the working Python decoder at
//! `creation_lib.renderer.sf_mesh_loader.parse_sf_mesh`.

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::skin::bone_remap::VertexInfluences;

#[derive(Debug)]
pub struct SfMeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub vertex_colors: Option<Vec<[u8; 4]>>,
    pub bone_influences: Option<Vec<VertexInfluences>>,
}

pub fn read_sf_mesh(path: &Path) -> Result<SfMeshData, String> {
    let data = std::fs::read(path)
        .map_err(|e| format!("sf mesh: failed to read {}: {e}", path.display()))?;
    parse_sf_mesh(&data)
}

pub fn resolve_geometry_path(geometries_root: &Path, mesh_id: &str) -> PathBuf {
    let normalized = mesh_id.replace('\\', "/");
    let trimmed = normalized
        .strip_suffix(".mesh")
        .or_else(|| normalized.strip_suffix(".MESH"))
        .unwrap_or(normalized.as_str());
    let mut path = geometries_root.to_path_buf();
    for part in trimmed.split('/').filter(|s| !s.is_empty()) {
        path.push(part);
    }
    path.set_extension("mesh");
    path
}

fn parse_sf_mesh(data: &[u8]) -> Result<SfMeshData, String> {
    let mut cur = Cursor::new(data);

    let version = read_u32(&mut cur, "version")?;
    if version > 2 {
        return Err(format!("sf mesh: unsupported version {version}"));
    }

    let indices_size = read_u32(&mut cur, "index count")? as usize;
    let num_tris = indices_size / 3;
    if num_tris == 0 {
        return Err("sf mesh: zero triangles".to_string());
    }
    let mut raw_indices = vec![0u16; indices_size];
    cur.read_u16_into::<LittleEndian>(&mut raw_indices)
        .map_err(|e| format!("sf mesh: truncated triangle indices: {e}"))?;
    let raw_triangles: Vec<[u32; 3]> = raw_indices
        .chunks_exact(3)
        .map(|c| [c[0] as u32, c[1] as u32, c[2] as u32])
        .collect();

    let scale = cur
        .read_f32::<LittleEndian>()
        .map_err(|e| format!("sf mesh: truncated at scale: {e}"))?;
    let weights_per_vert = read_u32(&mut cur, "weights_per_vert")? as usize;
    let num_positions = read_u32(&mut cur, "position count")? as usize;
    if !scale.is_finite() || scale <= 0.0 || num_positions == 0 {
        return Err("sf mesh: invalid scale or zero positions".to_string());
    }

    let mut raw_positions = vec![0i16; num_positions * 3];
    cur.read_i16_into::<LittleEndian>(&mut raw_positions)
        .map_err(|e| format!("sf mesh: truncated positions: {e}"))?;
    let positions: Vec<[f32; 3]> = raw_positions
        .chunks_exact(3)
        .map(|c| {
            [
                c[0] as f32 / 32767.0 * scale,
                c[1] as f32 / 32767.0 * scale,
                c[2] as f32 / 32767.0 * scale,
            ]
        })
        .collect();
    if raw_triangles
        .iter()
        .flatten()
        .any(|&index| index as usize >= num_positions)
    {
        return Err("sf mesh: triangle index exceeds position count".to_string());
    }

    let uvs = read_uv_channel(&mut cur, num_positions)?;
    let _uv2 = read_uv_channel(&mut cur, num_positions)?;

    let num_colors = read_u32(&mut cur, "color count")? as usize;
    let vertex_colors = if num_colors > 0 {
        let mut raw = vec![0u8; num_colors * 4];
        cur.read_exact(&mut raw)
            .map_err(|e| format!("sf mesh: truncated vertex colors: {e}"))?;
        Some(
            raw.chunks_exact(4)
                .map(|c| [c[2], c[1], c[0], c[3]]) // BGRA -> RGBA
                .collect(),
        )
    } else {
        None
    };

    let num_normals = read_u32(&mut cur, "normal count")? as usize;
    let normals = if num_normals > 0 {
        let mut raw = vec![0u32; num_normals];
        cur.read_u32_into::<LittleEndian>(&mut raw)
            .map_err(|e| format!("sf mesh: truncated normals: {e}"))?;
        raw.iter().map(|&p| decode_udec_normal(p)).collect()
    } else {
        compute_normals(&positions, &raw_triangles)
    };

    let num_tangents = read_u32(&mut cur, "tangent count")? as usize;
    let tangent_bytes = num_tangents
        .checked_mul(4)
        .filter(|&size| size <= data.len().saturating_sub(cur.position() as usize))
        .ok_or_else(|| "sf mesh: truncated tangents".to_string())?;
    cur.set_position(cur.position() + tangent_bytes as u64);

    let num_weights = read_u32(&mut cur, "weight count")? as usize;
    if num_positions.checked_mul(weights_per_vert) != Some(num_weights) {
        return Err("sf mesh: weight count does not match vertex count and stride".to_string());
    }
    let bone_influences = if weights_per_vert > 0 {
        if num_weights
            .checked_mul(4)
            .is_none_or(|size| size > data.len().saturating_sub(cur.position() as usize))
        {
            return Err("sf mesh: truncated bone weights".to_string());
        }
        let mut vertices = Vec::with_capacity(num_positions);
        for _ in 0..num_positions {
            let mut slots = Vec::with_capacity(weights_per_vert);
            for _ in 0..weights_per_vert {
                let bone = cur.read_u16::<LittleEndian>().unwrap() as usize;
                let weight = cur.read_u16::<LittleEndian>().unwrap();
                if weight != 0 {
                    slots.push((bone, weight as f32 / 65535.0));
                }
            }
            vertices.push(VertexInfluences { slots });
        }
        Some(vertices)
    } else {
        None
    };

    // Starfield winding is flipped relative to FO4 (sf_backend.py:260) —
    // swap indices 1 and 2 so the exported triangles cull correctly under FO4.
    let triangles: Vec<[u32; 3]> = raw_triangles.iter().map(|t| [t[0], t[2], t[1]]).collect();

    Ok(SfMeshData {
        positions,
        normals,
        uvs,
        triangles,
        vertex_colors,
        bone_influences,
    })
}

fn read_u32(cur: &mut Cursor<&[u8]>, field: &str) -> Result<u32, String> {
    cur.read_u32::<LittleEndian>()
        .map_err(|e| format!("sf mesh: truncated at {field}: {e}"))
}

fn read_uv_channel(cur: &mut Cursor<&[u8]>, num_positions: usize) -> Result<Vec<[f32; 2]>, String> {
    let count = read_u32(cur, "uv count")? as usize;
    if count == 0 {
        return Ok(vec![[0.0f32; 2]; num_positions]);
    }
    let mut raw = vec![0u16; count * 2];
    cur.read_u16_into::<LittleEndian>(&mut raw)
        .map_err(|e| format!("sf mesh: truncated uv data: {e}"))?;
    let mut uvs: Vec<[f32; 2]> = raw
        .chunks_exact(2)
        .map(|c| [half_to_f32(c[0]), half_to_f32(c[1])])
        .collect();
    if uvs.len() < num_positions {
        uvs.resize(num_positions, [0.0, 0.0]);
    }
    Ok(uvs)
}

fn decode_udec_normal(packed: u32) -> [f32; 3] {
    let x = (packed & 0x3FF) as f32 / 511.5 - 1.0;
    let y = ((packed >> 10) & 0x3FF) as f32 / 511.5 - 1.0;
    let z = ((packed >> 20) & 0x3FF) as f32 / 511.5 - 1.0;
    [x, y, z]
}

fn compute_normals(positions: &[[f32; 3]], triangles: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0f32; 3]; positions.len()];
    for tri in triangles {
        let v0 = positions[tri[0] as usize];
        let v1 = positions[tri[1] as usize];
        let v2 = positions[tri[2] as usize];
        let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let face_normal = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &idx in tri {
            let n = &mut normals[idx as usize];
            n[0] += face_normal[0];
            n[1] += face_normal[1];
            n[2] += face_normal[2];
        }
    }
    for n in normals.iter_mut() {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let len = if len < 1e-8 { 1.0 } else { len };
        n[0] /= len;
        n[1] /= len;
        n[2] /= len;
    }
    normals
}

// Public-domain half-precision decode (Jeroen van der Zijp-style table-free variant).
fn half_to_f32(bits: u16) -> f32 {
    let sign: u32 = (bits as u32 & 0x8000) << 16;
    let mut exponent: i32 = ((bits & 0x7C00) >> 10) as i32;
    let mut mantissa: u32 = (bits & 0x03FF) as u32;

    if exponent == 0 {
        if mantissa == 0 {
            return f32::from_bits(sign);
        }
        exponent = 1;
        while mantissa & 0x0400 == 0 {
            mantissa <<= 1;
            exponent -= 1;
        }
        mantissa &= 0x03FF;
        return f32::from_bits(sign | (((exponent + 112) as u32) << 23) | (mantissa << 13));
    }
    if exponent == 0x1F {
        return f32::from_bits(sign | 0x7F80_0000 | (mantissa << 13));
    }
    f32::from_bits(sign | (((exponent + 112) as u32) << 23) | (mantissa << 13))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_to_f32_matches_known_values() {
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0xBC00), -1.0);
        assert_eq!(half_to_f32(0x0000), 0.0);
    }

    #[test]
    fn decode_udec_normal_recovers_unit_axes() {
        let packed = 511u32 | (511u32 << 10) | (1023u32 << 20);
        let n = decode_udec_normal(packed);
        assert!((n[0] - 0.0).abs() < 1e-3);
        assert!((n[1] - 0.0).abs() < 1e-3);
        assert!((n[2] - 1.0).abs() < 1e-3);
        assert_eq!(decode_udec_normal(0), [-1.0; 3]);
        assert_eq!(decode_udec_normal(0x3fffffff), [1.0; 3]);
        assert!(
            decode_udec_normal(0x5ff7fdff)
                .iter()
                .all(|value| *value < 0.0 && *value > -0.001)
        );
    }

    #[test]
    fn triangle_winding_is_flipped() {
        let data = build_minimal_mesh();
        let mesh = parse_sf_mesh(&data).expect("parses");
        assert_eq!(mesh.triangles[0], [0, 2, 1]);
        assert!(mesh.bone_influences.is_none());
    }

    #[test]
    fn skin_weights_preserve_vertex_major_bone_indices_and_unorm_values() {
        let mut data = build_minimal_mesh();
        data[18..22].copy_from_slice(&2u32.to_le_bytes());
        let count_offset = data.len() - 4;
        data[count_offset..].copy_from_slice(&6u32.to_le_bytes());
        for (bone, weight) in [
            (2u16, 49151u16),
            (5, 16384),
            (7, 65535),
            (0, 0),
            (1, 0),
            (9, 65535),
        ] {
            data.extend_from_slice(&bone.to_le_bytes());
            data.extend_from_slice(&weight.to_le_bytes());
        }
        let mesh = parse_sf_mesh(&data).unwrap();
        let influences = mesh.bone_influences.unwrap();
        assert_eq!(influences[0].slots.len(), 2);
        assert_eq!(influences[0].slots[0].0, 2);
        assert_eq!(influences[0].slots[1].0, 5);
        assert!((influences[0].slots[0].1 - 0.75).abs() < 0.00002);
        assert_eq!(influences[1].slots, vec![(7, 1.0)]);
        assert_eq!(influences[2].slots, vec![(9, 1.0)]);
    }

    #[test]
    fn malformed_skin_weight_count_and_truncated_payload_are_rejected() {
        let mut data = build_minimal_mesh();
        data[18..22].copy_from_slice(&2u32.to_le_bytes());
        assert!(parse_sf_mesh(&data).unwrap_err().contains("weight count"));
        let count_offset = data.len() - 4;
        data[count_offset..].copy_from_slice(&6u32.to_le_bytes());
        assert!(
            parse_sf_mesh(&data)
                .unwrap_err()
                .contains("truncated bone weights")
        );
    }

    #[test]
    fn resolve_geometry_path_joins_hash_components() {
        let root = Path::new("fake/geometries");
        let resolved = resolve_geometry_path(root, r"0024965a94937a847041\f66e90898c3334f02c7d");
        assert_eq!(
            resolved,
            root.join("0024965a94937a847041")
                .join("f66e90898c3334f02c7d.mesh")
        );
    }

    #[test]
    fn resolve_geometry_path_strips_existing_extension() {
        let root = Path::new("fake/geometries");
        let resolved = resolve_geometry_path(root, "abc/def.mesh");
        assert_eq!(resolved, root.join("abc").join("def.mesh"));
    }

    #[test]
    #[ignore = "requires extracted Starfield outfit geometries"]
    fn real_outfit_packed_normals_remain_finite_and_nonzero() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/starfield/geometries");
        let mut rows = Vec::new();
        for path in [
            "7bf9fa063821f7b581e5/8bbf9073e4f77b120c39",
            "23d53ad50e363004ec50/fac260a94e7307230438",
            "ea36a18744b551faecb8/bb3ba8a77f6e9faa6f9e",
        ] {
            let mesh = read_sf_mesh(&resolve_geometry_path(&root, path)).unwrap();
            let invalid = mesh
                .normals
                .iter()
                .enumerate()
                .filter(|(_, n)| n.iter().map(|v| v * v).sum::<f32>() < 1e-12)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            assert_eq!(mesh.normals.len(), mesh.positions.len());
            assert!(mesh.normals.iter().flatten().all(|value| value.is_finite()));
            rows.push(serde_json::json!({"mesh":path,"vertices":mesh.positions.len(),"zero_normals":invalid}));
        }
        let receipt = std::env::temp_dir().join("b21_starfield_zero_normal_receipt.json");
        std::fs::write(&receipt, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
        println!("{}", receipt.display());
        assert!(
            rows.iter()
                .all(|row| row["zero_normals"].as_array().unwrap().is_empty()),
            "{rows:?}"
        );
    }

    fn build_minimal_mesh() -> Vec<u8> {
        // One triangle, 3 vertices, no uvs/uv2/colors/normals/tangents.
        let mut buf = Vec::new();
        buf.extend_from_slice(&2u32.to_le_bytes()); // version
        buf.extend_from_slice(&3u32.to_le_bytes()); // indices_size
        for idx in [0u16, 1u16, 2u16] {
            buf.extend_from_slice(&idx.to_le_bytes());
        }
        buf.extend_from_slice(&1.0f32.to_le_bytes()); // scale
        buf.extend_from_slice(&0u32.to_le_bytes()); // weights_per_vert
        buf.extend_from_slice(&3u32.to_le_bytes()); // num_positions
        for v in [[0i16, 0, 0], [10000i16, 0, 0], [0i16, 10000, 0]] {
            for c in v {
                buf.extend_from_slice(&c.to_le_bytes());
            }
        }
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_uv1
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_uv2
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_colors
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_normals
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_tangents
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_weights
        buf
    }
}
