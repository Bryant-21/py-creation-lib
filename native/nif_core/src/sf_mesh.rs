//! Starfield `.mesh` binary reader — external geometry for `BSGeometry` blocks.
//!
//! Layout ported 1:1 from the working Python decoder at
//! `creation_lib.renderer.sf_mesh_loader.parse_sf_mesh`.

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

pub struct SfMeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub vertex_colors: Option<Vec<[u8; 4]>>,
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
    let _weights_per_vert = read_u32(&mut cur, "weights_per_vert")?;
    let num_positions = read_u32(&mut cur, "position count")? as usize;
    if scale <= 0.0 || num_positions == 0 {
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

    // Starfield winding is flipped relative to FO4 (sf_backend.py:260) —
    // swap indices 1 and 2 so the exported triangles cull correctly under FO4.
    let triangles: Vec<[u32; 3]> = raw_triangles.iter().map(|t| [t[0], t[2], t[1]]).collect();

    Ok(SfMeshData {
        positions,
        normals,
        uvs,
        triangles,
        vertex_colors,
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
    let x = (packed & 0x3FF) as f32 / 511.0 - 1.0;
    let y = ((packed >> 10) & 0x3FF) as f32 / 511.0 - 1.0;
    let z = ((packed >> 20) & 0x3FF) as f32 / 511.0 - 1.0;
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
        // 511 => (511/511 - 1) = 0.0; 1022 => (1022/511 - 1) = 1.0
        let packed = 511u32 | (511u32 << 10) | (1022u32 << 20);
        let n = decode_udec_normal(packed);
        assert!((n[0] - 0.0).abs() < 1e-3);
        assert!((n[1] - 0.0).abs() < 1e-3);
        assert!((n[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn triangle_winding_is_flipped() {
        let data = build_minimal_mesh();
        let mesh = parse_sf_mesh(&data).expect("parses");
        assert_eq!(mesh.triangles[0], [0, 2, 1]);
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
        buf
    }
}
