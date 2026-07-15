use super::types::{LandHeightMap, LandVertexNormals};
use thiserror::Error;

pub const VHGT_SIZE: usize = 1096;
pub const VNML_SIZE: usize = 3267;
const GRID: usize = 33;

#[derive(Debug, Error)]
pub enum LandError {
    #[error("VHGT truncated: need 1096 bytes, have {0}")]
    VhgtTruncated(usize),
    #[error("VNML truncated: need 3267 bytes, have {0}")]
    VnmlTruncated(usize),
    #[error("VHGT has {0} trailing bytes (need exactly 1096)")]
    VhgtTrailing(usize),
    #[error("VNML has {0} trailing bytes (need exactly 3267)")]
    VnmlTrailing(usize),
    #[error("LAND parse: {0}")]
    Other(String),
}

pub fn parse_heightmap(bytes: &[u8]) -> Result<LandHeightMap, LandError> {
    if bytes.len() < VHGT_SIZE {
        return Err(LandError::VhgtTruncated(bytes.len()));
    }
    if bytes.len() > VHGT_SIZE {
        return Err(LandError::VhgtTrailing(bytes.len() - VHGT_SIZE));
    }
    let base = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let mut deltas = [[0i8; GRID]; GRID];
    for r in 0..GRID {
        for c in 0..GRID {
            deltas[r][c] = bytes[4 + r * GRID + c] as i8;
        }
    }
    // 3 trailing padding bytes at offset 1093..1096 — preserved verbatim by
    // the writer (always zero in observed corpus data). We don't validate
    // their value here; byte-roundtrip is enforced by tests.
    Ok(LandHeightMap { base, deltas })
}

pub fn parse_vertex_normals(bytes: &[u8]) -> Result<LandVertexNormals, LandError> {
    if bytes.len() < VNML_SIZE {
        return Err(LandError::VnmlTruncated(bytes.len()));
    }
    if bytes.len() > VNML_SIZE {
        return Err(LandError::VnmlTrailing(bytes.len() - VNML_SIZE));
    }
    let mut normals = [[(0i8, 0i8, 0i8); GRID]; GRID];
    for r in 0..GRID {
        for c in 0..GRID {
            let off = r * GRID * 3 + c * 3;
            normals[r][c] = (bytes[off] as i8, bytes[off + 1] as i8, bytes[off + 2] as i8);
        }
    }
    Ok(LandVertexNormals { normals })
}
