use super::parser::{VHGT_SIZE, VNML_SIZE};
use super::types::{LandHeightMap, LandVertexNormals};

const GRID: usize = 33;

pub fn write_heightmap(map: &LandHeightMap) -> Vec<u8> {
    let mut out = Vec::with_capacity(VHGT_SIZE);
    out.extend_from_slice(&map.base.to_le_bytes());
    for r in 0..GRID {
        for c in 0..GRID {
            out.push(map.deltas[r][c] as u8);
        }
    }
    out.extend_from_slice(&[0u8; 3]);
    out
}

pub fn write_vertex_normals(n: &LandVertexNormals) -> Vec<u8> {
    let mut out = Vec::with_capacity(VNML_SIZE);
    for r in 0..GRID {
        for c in 0..GRID {
            let (x, y, z) = n.normals[r][c];
            out.push(x as u8);
            out.push(y as u8);
            out.push(z as u8);
        }
    }
    out
}
