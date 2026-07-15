/// `.lod` LODSettings file writer.
///
/// Ported from the FO4 (non-Fallout3) branch of `TwbLodSettings.LoadFromData`
/// (`wbLOD.pas:432-459`, R3 §1). The format is a fixed 16-byte little-endian record:
///   [i16 SWx][i16 SWy][i32 Stride][i32 Min][i32 Max]

/// Encode a `.lod` file into its 16-byte little-endian wire format.
pub fn encode(sw: (i32, i32), stride: i32, min: i32, max: i32) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..2].copy_from_slice(&(sw.0 as i16).to_le_bytes());
    buf[2..4].copy_from_slice(&(sw.1 as i16).to_le_bytes());
    buf[4..8].copy_from_slice(&stride.to_le_bytes());
    buf[8..12].copy_from_slice(&min.to_le_bytes());
    buf[12..16].copy_from_slice(&max.to_le_bytes());
    buf
}

/// Compute the stride for a worldspace: next power of two of max(ne - sw) span (R3 §1).
pub fn next_stride(sw: (i32, i32), ne: (i32, i32)) -> i32 {
    let span_x = (ne.0 - sw.0).unsigned_abs();
    let span_y = (ne.1 - sw.1).unsigned_abs();
    let span = span_x.max(span_y);
    if span == 0 {
        return 1;
    }
    span.next_power_of_two() as i32
}

/// Write a `.lod` file to `path`, creating parent directories as needed.
pub fn write(
    path: &std::path::Path,
    sw: (i32, i32),
    stride: i32,
    min: i32,
    max: i32,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, encode(sw, stride, min, max))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_16_bytes_le() {
        // SWx=-25, SWy=-27, Stride=64, Min=4, Max=32
        let bytes = encode((-25, -27), 64, 4, 32);
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[0..2], &(-25i16).to_le_bytes());
        assert_eq!(&bytes[2..4], &(-27i16).to_le_bytes());
        assert_eq!(&bytes[4..8], &64i32.to_le_bytes());
        assert_eq!(&bytes[8..12], &4i32.to_le_bytes());
        assert_eq!(&bytes[12..16], &32i32.to_le_bytes());
    }

    #[test]
    fn stride_is_next_pow2_of_span() {
        // span = max(ne-sw) ; next pow2 (R3 §1 GetSize uses Ceil(Stride/sqrt2))
        assert_eq!(next_stride((0, 0), (40, 30)), 64); // span 40 -> 64
        assert_eq!(next_stride((-16, -16), (16, 16)), 32); // span 32 -> 32
    }

    #[test]
    fn write_roundtrips_to_disk() {
        let dir = std::env::temp_dir().join("lodgen_lod_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("W.lod");
        write(&p, (-25, -27), 64, 4, 32).unwrap();
        let data = std::fs::read(&p).unwrap();
        assert_eq!(data, encode((-25, -27), 64, 4, 32).to_vec());
    }
}
