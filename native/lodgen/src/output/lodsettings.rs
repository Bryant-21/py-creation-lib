/// `.lod` LODSettings file writer.
///
/// Ported from the FO4 (non-Fallout3) branch of `TwbLodSettings.LoadFromData`
/// (`wbLOD.pas:432-459`, R3 §1). The format is a fixed 16-byte little-endian record:
///   [i16 SWx][i16 SWy][i32 Stride][i32 Min][i32 Max]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLodSettings {
    pub southwest: (i32, i32),
    pub northeast: Option<(i32, i32)>,
    pub stride: i32,
    pub min: i32,
    pub max: i32,
    pub object_level: Option<i32>,
}

fn validate_source(settings: SourceLodSettings) -> anyhow::Result<SourceLodSettings> {
    if settings.stride <= 0 {
        anyhow::bail!(
            "LOD settings stride must be positive, got {}",
            settings.stride
        );
    }
    if settings.min <= 0 || settings.max < settings.min {
        anyhow::bail!(
            "LOD settings level range is invalid: {}..{}",
            settings.min,
            settings.max
        );
    }
    if let Some(ne) = settings.northeast {
        if ne.0 < settings.southwest.0 || ne.1 < settings.southwest.1 {
            anyhow::bail!(
                "LOD settings bounds are invalid: southwest {:?}, northeast {:?}",
                settings.southwest,
                ne
            );
        }
    }
    Ok(settings)
}

pub fn decode_source(bytes: &[u8]) -> anyhow::Result<SourceLodSettings> {
    let i16_at = |offset: usize| -> anyhow::Result<i16> {
        let value = bytes
            .get(offset..offset + 2)
            .ok_or_else(|| anyhow::anyhow!("LOD settings truncated at byte {offset}"))?;
        Ok(i16::from_le_bytes(value.try_into().unwrap()))
    };
    let i32_at = |offset: usize| -> anyhow::Result<i32> {
        let value = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| anyhow::anyhow!("LOD settings truncated at byte {offset}"))?;
        Ok(i32::from_le_bytes(value.try_into().unwrap()))
    };

    let settings = match bytes.len() {
        16 => SourceLodSettings {
            southwest: (i16_at(0)? as i32, i16_at(2)? as i32),
            northeast: None,
            stride: i32_at(4)?,
            min: i32_at(8)?,
            max: i32_at(12)?,
            object_level: None,
        },
        24 => SourceLodSettings {
            southwest: (i16_at(12)? as i32, i16_at(14)? as i32),
            northeast: Some((i16_at(16)? as i32, i16_at(18)? as i32)),
            stride: i32_at(8)?,
            min: i32_at(0)?,
            max: i32_at(4)?,
            object_level: Some(i32_at(20)?),
        },
        length => anyhow::bail!(
            "unsupported LOD settings length {length}; expected 16-byte .lod or 24-byte .dlodsettings"
        ),
    };
    validate_source(settings)
}

fn child_case_insensitive(parent: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let exact = parent.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        })
        .map(|entry| entry.path())
}

pub fn read_source(
    source_data_dir: &std::path::Path,
    world: &str,
) -> anyhow::Result<Option<SourceLodSettings>> {
    let Some(settings_dir) = child_case_insensitive(source_data_dir, "LODSettings") else {
        return Ok(None);
    };
    let lod_path = child_case_insensitive(&settings_dir, &format!("{world}.lod"));
    let dlod_path = child_case_insensitive(&settings_dir, &format!("{world}.dlodsettings"));
    let (path, expected_len) = match (lod_path, dlod_path) {
        (Some(lod), Some(dlod)) => anyhow::bail!(
            "ambiguous source LOD settings for {world}: {} and {}",
            lod.display(),
            dlod.display()
        ),
        (Some(path), None) => (path, 16),
        (None, Some(path)) => (path, 24),
        (None, None) => return Ok(None),
    };
    let bytes = std::fs::read(&path)?;
    if bytes.len() != expected_len {
        anyhow::bail!(
            "{}: expected {expected_len} bytes, got {}",
            path.display(),
            bytes.len()
        );
    }
    decode_source(&bytes)
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{}: {error}", path.display()))
}

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

    #[test]
    fn decode_skyrim_lod_settings() {
        let bytes = encode((-96, -96), 256, 4, 32);
        assert_eq!(
            decode_source(&bytes).unwrap(),
            SourceLodSettings {
                southwest: (-96, -96),
                northeast: None,
                stride: 256,
                min: 4,
                max: 32,
                object_level: None,
            }
        );
    }

    #[test]
    fn decode_fnv_dlodsettings() {
        let bytes = [
            4, 0, 0, 0, 32, 0, 0, 0, 128, 0, 0, 0, 0xC0, 0xFF, 0xC0, 0xFF, 0x3F, 0, 0x3F, 0, 4, 0,
            0, 0,
        ];
        assert_eq!(
            decode_source(&bytes).unwrap(),
            SourceLodSettings {
                southwest: (-64, -64),
                northeast: Some((63, 63)),
                stride: 128,
                min: 4,
                max: 32,
                object_level: Some(4),
            }
        );
    }

    #[test]
    fn read_source_matches_case_insensitively() {
        let root =
            std::env::temp_dir().join(format!("lodgen_source_settings_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let settings_dir = root.join("lodsettings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(
            settings_dir.join("tamriel.LOD"),
            encode((-96, -96), 256, 4, 32),
        )
        .unwrap();

        let settings = read_source(&root, "Tamriel").unwrap().unwrap();
        assert_eq!(settings.southwest, (-96, -96));
        assert_eq!(settings.stride, 256);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_source_rejects_extension_size_mismatch() {
        let root = std::env::temp_dir().join(format!(
            "lodgen_source_settings_size_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let settings_dir = root.join("LODSettings");
        std::fs::create_dir_all(&settings_dir).unwrap();
        std::fs::write(
            settings_dir.join("Tamriel.dlodsettings"),
            encode((-96, -96), 256, 4, 32),
        )
        .unwrap();

        let error = read_source(&root, "Tamriel").unwrap_err().to_string();
        assert!(error.contains("expected 24 bytes, got 16"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
