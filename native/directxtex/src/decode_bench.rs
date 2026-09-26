use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "directxtex_decode_{label}_{}_{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ))
}

fn patterned_rgba(width: u32, height: u32) -> Vec<u8> {
    (0..width as usize * height as usize)
        .flat_map(|i| {
            let x = (i % width as usize) as u8;
            let y = (i / width as usize) as u8;
            [
                x.wrapping_mul(17).wrapping_add(y.wrapping_mul(3)),
                x.wrapping_mul(5).wrapping_add(y.wrapping_mul(19)),
                x.wrapping_mul(11).wrapping_add(y.wrapping_mul(7)),
                if i % 5 == 0 { 0 } else { 255 },
            ]
        })
        .collect()
}

fn baseline_full_image_decode(
    bytes: &[u8],
) -> core::result::Result<(u32, u32, Vec<u8>, u32), String> {
    if let Some(decoded) = try_load_legacy_rgba8_dds(bytes)? {
        return Ok(decoded);
    }
    let scratch = ScratchImage::load_dds(bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
        .map_err(|err| err.to_string())?;
    let metadata = *scratch.metadata();
    let width = metadata
        .width
        .try_into()
        .map_err(|_| "dds width exceeds u32".to_string())?;
    let height = metadata
        .height
        .try_into()
        .map_err(|_| "dds height exceeds u32".to_string())?;
    let dxgi_format = metadata.format.bits();

    if metadata.is_cubemap()
        && matches!(
            metadata.format,
            DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
                | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
        )
    {
        let image = scratch
            .image(0, 0, 0)
            .ok_or_else(|| "dds cubemap does not contain a readable base face".to_string())?;
        let rgba = packed_pixels_from_image(
            image,
            true,
            matches!(
                metadata.format,
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
                    | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM_SRGB
            ),
        )?;
        return Ok((width, height, rgba, dxgi_format));
    }

    if metadata.format == DXGI_FORMAT::DXGI_FORMAT_BC5_UNORM {
        let image = scratch
            .image(0, 0, 0)
            .ok_or_else(|| "dds BC5 image does not contain a readable base mip".to_string())?;
        let rgba = decode_bc5_unorm_image(image)?;
        return Ok((width, height, rgba, dxgi_format));
    }

    let rgba_scratch = if metadata.format == DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM {
        None
    } else if metadata.format.is_compressed() {
        Some(
            scratch
                .decompress(DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM)
                .map_err(|err| err.to_string())?,
        )
    } else {
        Some(
            scratch
                .convert(
                    DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                    TEX_FILTER_FLAGS::TEX_FILTER_DEFAULT,
                    TEX_THRESHOLD_DEFAULT,
                )
                .map_err(|err| err.to_string())?,
        )
    };
    let image = rgba_scratch
        .as_ref()
        .unwrap_or(&scratch)
        .image(0, 0, 0)
        .ok_or_else(|| "dds conversion did not produce an RGBA image".to_string())?;
    let rgba = packed_pixels_from_image(image, false, false)?;
    Ok((width, height, rgba, dxgi_format))
}

fn assert_decode_and_reencode_parity(bytes: &[u8], label: &str) -> bool {
    let baseline = baseline_full_image_decode(bytes);
    let candidate = dds_base_rgba_bytes(bytes);
    let (baseline, candidate) = match (baseline, candidate) {
        (Ok(baseline), Ok(candidate)) => (baseline, candidate),
        (Err(baseline), Err(candidate)) => {
            assert_eq!(candidate, baseline, "decode error mismatch for {label}");
            eprintln!("matching unsupported decode for {label}: {baseline}");
            return false;
        }
        (baseline, candidate) => panic!(
            "decode result mismatch for {label}: baseline={baseline:?} candidate={candidate:?}"
        ),
    };
    assert_eq!(candidate, baseline, "decoded base mismatch for {label}");

    let baseline_path = temp_path(&format!("{label}_baseline.dds"));
    let candidate_path = temp_path(&format!("{label}_candidate.dds"));
    for (path, decoded) in [(&baseline_path, &baseline), (&candidate_path, &candidate)] {
        write_dds_rgba_image(path, decoded.0, decoded.1, &decoded.2, "BC7_UNORM", true).unwrap();
    }
    assert_eq!(
        std::fs::read(&candidate_path).unwrap(),
        std::fs::read(&baseline_path).unwrap(),
        "re-encoded DDS mismatch for {label}"
    );
    let _ = std::fs::remove_file(baseline_path);
    let _ = std::fs::remove_file(candidate_path);
    true
}

fn write_plain_fixture(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    format: &str,
    generate_mips: bool,
) -> core::result::Result<(), String> {
    if legacy_dxt_target(format).is_some() {
        return write_dds_rgba_image(path, width, height, rgba, format, generate_mips);
    }
    let target =
        parse_dxgi_format(format).ok_or_else(|| format!("unknown fixture format {format}"))?;
    if !matches!(
        target,
        DXGI_FORMAT::DXGI_FORMAT_R8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_R8G8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_B8G8R8X8_UNORM
            | DXGI_FORMAT::DXGI_FORMAT_R16G16B16A16_FLOAT
    ) {
        return write_dds_rgba_image(path, width, height, rgba, format, generate_mips);
    }

    let mut scratch = ScratchImage::default();
    scratch
        .initialize_2d(
            target,
            width as usize,
            height as usize,
            1,
            if generate_mips {
                full_mip_count(width, height) as usize
            } else {
                1
            },
            CP_FLAGS::CP_FLAGS_NONE,
        )
        .map_err(|error| error.to_string())?;
    if target == DXGI_FORMAT::DXGI_FORMAT_R16G16B16A16_FLOAT {
        let finite_half_values = [0x0000u16, 0x3400, 0x3800, 0x3c00];
        for (index, value) in scratch.pixels_mut().chunks_exact_mut(2).enumerate() {
            value.copy_from_slice(
                &finite_half_values[index % finite_half_values.len()].to_le_bytes(),
            );
        }
    } else {
        for (index, value) in scratch.pixels_mut().iter_mut().enumerate() {
            *value = index.wrapping_mul(31).wrapping_add(13) as u8;
        }
    }
    let encoded = scratch
        .save_dds(DDS_FLAGS::DDS_FLAGS_NONE)
        .map_err(|error| error.to_string())?;
    std::fs::write(path, encoded.buffer()).map_err(|error| error.to_string())
}

#[test]
fn base_only_decode_matches_full_image_decode_for_supported_formats() {
    let rgba = patterned_rgba(32, 16);
    for format in [
        "DXT1",
        "DXT5",
        "R8_UNORM",
        "R8G8_UNORM",
        "B8G8R8A8_UNORM",
        "B8G8R8X8_UNORM",
        "R16G16B16A16_FLOAT",
        "BC1_UNORM",
        "BC1_UNORM_SRGB",
        "BC2_UNORM",
        "BC2_UNORM_SRGB",
        "BC3_UNORM",
        "BC3_UNORM_SRGB",
        "BC4_UNORM",
        "BC4_SNORM",
        "BC5_UNORM",
        "BC5_SNORM",
        "BC6H_UF16",
        "BC6H_SF16",
        "BC7_UNORM",
        "BC7_UNORM_SRGB",
        "R8G8B8A8_UNORM",
        "R8G8B8A8_UNORM_SRGB",
    ] {
        for generate_mips in [false, true] {
            let path = temp_path(&format!("source_{format}_{generate_mips}.dds"));
            write_plain_fixture(&path, 32, 16, &rgba, format, generate_mips).unwrap_or_else(
                |error| panic!("fixture writer rejected {format} mips={generate_mips}: {error}"),
            );
            let bytes = std::fs::read(&path).unwrap();
            let decoded =
                assert_decode_and_reencode_parity(&bytes, &format!("{format}_{generate_mips}"));
            assert!(
                decoded || format == "R8_UNORM",
                "plain fixture must decode: {format} mips={generate_mips}"
            );
            let _ = std::fs::remove_file(path);
        }
    }
}

fn complex_dds(kind: &str) -> Vec<u8> {
    let mut source = ScratchImage::default();
    match kind {
        "array" => source
            .initialize_2d(
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM,
                16,
                8,
                2,
                3,
                CP_FLAGS::CP_FLAGS_NONE,
            )
            .unwrap(),
        "cubemap" => source
            .initialize_cube(
                DXGI_FORMAT::DXGI_FORMAT_R8G8B8A8_UNORM,
                16,
                16,
                1,
                3,
                CP_FLAGS::CP_FLAGS_NONE,
            )
            .unwrap(),
        "volume" | "volume_depth1" => source
            .initialize_3d(
                DXGI_FORMAT::DXGI_FORMAT_B8G8R8A8_UNORM,
                16,
                8,
                if kind == "volume" { 4 } else { 1 },
                3,
                CP_FLAGS::CP_FLAGS_NONE,
            )
            .unwrap(),
        _ => unreachable!(),
    }
    for (i, byte) in source.pixels_mut().iter_mut().enumerate() {
        *byte = i.wrapping_mul(29).wrapping_add(7) as u8;
    }
    let encoded_source = if kind == "cubemap" {
        source
            .compress(
                DXGI_FORMAT::DXGI_FORMAT_BC1_UNORM,
                TEX_COMPRESS_FLAGS::TEX_COMPRESS_DEFAULT,
                TEX_THRESHOLD_DEFAULT,
            )
            .unwrap_or_else(|error| panic!("complex fixture conversion rejected {kind}: {error}"))
    } else {
        source
    };
    encoded_source
        .save_dds(DDS_FLAGS::DDS_FLAGS_NONE)
        .unwrap()
        .buffer()
        .to_vec()
}

#[test]
fn complex_resources_keep_full_image_fallback() {
    for kind in ["array", "cubemap", "volume", "volume_depth1"] {
        let bytes = complex_dds(kind);
        let metadata = ScratchImage::load_dds(&bytes, DDS_FLAGS::DDS_FLAGS_NONE, None, None)
            .unwrap()
            .metadata()
            .to_owned();
        assert!(
            metadata.dimension != TEX_DIMENSION::TEX_DIMENSION_TEXTURE2D
                || metadata.is_cubemap()
                || metadata.array_size > 1
                || metadata.depth > 1,
            "fixture is not complex: {kind}"
        );
        let _ = assert_decode_and_reencode_parity(&bytes, kind);
    }
}

#[test]
fn malformed_complete_load_errors_before_base_decode() {
    let path = temp_path("truncated_source.dds");
    let rgba = patterned_rgba(32, 32);
    write_dds_rgba_image(&path, 32, 32, &rgba, "BC7_UNORM", true).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 1);
    assert!(baseline_full_image_decode(&bytes).is_err());
    assert!(dds_base_rgba_bytes(&bytes).is_err());
    let _ = std::fs::remove_file(path);
}
