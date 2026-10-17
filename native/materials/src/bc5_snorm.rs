//! BC5_SNORM to BC5_UNORM without re-encoding for source-game normal maps.
//!
//! A BC4 block's palette is a linear function of its two endpoints, so shifting each signed
//! endpoint by +128 (XOR 0x80) moves every texel and leaves the index bits alone. After FO4's
//! `2x - 1` expansion, s/127 becomes (2s + 1)/255: an error of (127 - s)/32385, at most 1/127.5.
//! The unsigned order of the shifted endpoints equals the signed order of the originals, so every
//! block keeps its interpolation mode.

use thiserror::Error;

pub const DXGI_FORMAT_BC5_UNORM: u32 = 83;
pub const DXGI_FORMAT_BC5_SNORM: u32 = 84;

const BLOCK_SIZE: usize = 16;
const ENDPOINT_BYTES: [usize; 4] = [0, 1, 8, 9];
const FOURCC_OFFSET: usize = 84;
const DXGI_FORMAT_OFFSET: usize = 128;
const DX10_DATA_OFFSET: usize = 148;
const LEGACY_DATA_OFFSET: usize = 128;

#[derive(Debug, PartialEq, Eq, Error)]
pub enum Bc5ShiftError {
    #[error("BC5 data of {len} bytes is not a whole number of 16-byte blocks")]
    NotWholeBlocks { len: usize },
}

pub fn bc5_snorm_to_unorm_in_place(blocks: &mut [u8]) -> Result<(), Bc5ShiftError> {
    if blocks.len() % BLOCK_SIZE != 0 {
        return Err(Bc5ShiftError::NotWholeBlocks { len: blocks.len() });
    }
    for block in blocks.chunks_exact_mut(BLOCK_SIZE) {
        for offset in ENDPOINT_BYTES {
            block[offset] ^= 0x80;
        }
    }
    Ok(())
}

pub fn shift_bc5_snorm_dds(dds: &mut [u8]) -> Result<bool, Bc5ShiftError> {
    if dds.len() < LEGACY_DATA_OFFSET || &dds[..4] != b"DDS " {
        return Ok(false);
    }
    if &dds[FOURCC_OFFSET..FOURCC_OFFSET + 4] == b"BC5S" {
        bc5_snorm_to_unorm_in_place(&mut dds[LEGACY_DATA_OFFSET..])?;
        dds[FOURCC_OFFSET..FOURCC_OFFSET + 4].copy_from_slice(b"ATI2");
        return Ok(true);
    }
    if dds.len() < DX10_DATA_OFFSET || &dds[FOURCC_OFFSET..FOURCC_OFFSET + 4] != b"DX10" {
        return Ok(false);
    }
    let format = u32::from_le_bytes([
        dds[DXGI_FORMAT_OFFSET],
        dds[DXGI_FORMAT_OFFSET + 1],
        dds[DXGI_FORMAT_OFFSET + 2],
        dds[DXGI_FORMAT_OFFSET + 3],
    ]);
    if format != DXGI_FORMAT_BC5_SNORM {
        return Ok(false);
    }
    bc5_snorm_to_unorm_in_place(&mut dds[DX10_DATA_OFFSET..])?;
    dds[DXGI_FORMAT_OFFSET..DXGI_FORMAT_OFFSET + 4]
        .copy_from_slice(&DXGI_FORMAT_BC5_UNORM.to_le_bytes());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture_convert::{
        TexturePathInput, TexturePathOutput, TextureSetPathRequest, convert_texture_set_paths,
    };

    fn block(red0: u8, red1: u8, green0: u8, green1: u8) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0] = red0;
        out[1] = red1;
        out[2..8].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);
        out[8] = green0;
        out[9] = green1;
        out[10..16].copy_from_slice(&[0xDE, 0xF0, 0x11, 0x22, 0x33, 0x44]);
        out
    }

    // The D3D BC4 palette in index order, from decoded endpoint values.
    fn palette(e0: f64, e1: f64, eight_values: bool, low: f64, high: f64) -> Vec<f64> {
        let mut values = vec![e0, e1];
        if eight_values {
            values.extend((1..7_i32).map(|i| (f64::from(7 - i) * e0 + f64::from(i) * e1) / 7.0));
        } else {
            values.extend((1..5_i32).map(|i| (f64::from(5 - i) * e0 + f64::from(i) * e1) / 5.0));
            values.extend([low, high]);
        }
        values
    }

    fn snorm_palette(e0: u8, e1: u8) -> (Vec<f64>, bool) {
        let (s0, s1) = (e0 as i8, e1 as i8);
        let decode = |s: i8| f64::from(s.max(-127)) / 127.0;
        let eight_values = s0 > s1;
        (
            palette(decode(s0), decode(s1), eight_values, -1.0, 1.0),
            eight_values,
        )
    }

    // What an FO4 shader sees: the UNORM value expanded with 2x - 1.
    fn unorm_palette_expanded(e0: u8, e1: u8) -> (Vec<f64>, bool) {
        let decode = |u: u8| f64::from(u) / 255.0;
        let eight_values = e0 > e1;
        let values = palette(decode(e0), decode(e1), eight_values, 0.0, 1.0);
        (
            values.into_iter().map(|v| 2.0 * v - 1.0).collect(),
            eight_values,
        )
    }

    fn dds(fourcc: &[u8; 4], format: u32, data: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; DX10_DATA_OFFSET];
        out[..4].copy_from_slice(b"DDS ");
        out[FOURCC_OFFSET..FOURCC_OFFSET + 4].copy_from_slice(fourcc);
        out[DXGI_FORMAT_OFFSET..DXGI_FORMAT_OFFSET + 4].copy_from_slice(&format.to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn shifts_only_the_endpoint_bytes() {
        let mut data = [block(0x81, 0x7F, 0x00, 0xFF), block(0x80, 0x80, 0x7F, 0x7F)].concat();
        bc5_snorm_to_unorm_in_place(&mut data).unwrap();
        assert_eq!(
            data,
            [block(0x01, 0xFF, 0x80, 0x7F), block(0x00, 0x00, 0xFF, 0xFF)].concat()
        );
    }

    #[test]
    fn refuses_data_that_is_not_whole_blocks() {
        let mut data = vec![0x80u8; 17];
        assert_eq!(
            bc5_snorm_to_unorm_in_place(&mut data),
            Err(Bc5ShiftError::NotWholeBlocks { len: 17 })
        );
        assert_eq!(data, vec![0x80u8; 17]);
    }

    #[test]
    fn every_endpoint_pair_keeps_its_mode_and_stays_within_the_bound() {
        let bound = 254.0 / 32385.0 + 1e-12;
        for e0 in 0..=255u8 {
            for e1 in 0..=255u8 {
                let (snorm, snorm_mode) = snorm_palette(e0, e1);
                let (unorm, unorm_mode) = unorm_palette_expanded(e0 ^ 0x80, e1 ^ 0x80);
                assert_eq!(snorm_mode, unorm_mode, "mode of {e0:#04x} {e1:#04x}");
                for (index, (s, u)) in snorm.iter().zip(&unorm).enumerate() {
                    assert!(
                        (s - u).abs() <= bound,
                        "{e0:#04x} {e1:#04x} entry {index}: {s} vs {u}"
                    );
                }
            }
        }
    }

    #[test]
    fn shifts_a_bc5_snorm_dds_and_relabels_it_unorm() {
        let mut file = dds(
            b"DX10",
            DXGI_FORMAT_BC5_SNORM,
            &block(0x81, 0x7F, 0x00, 0xFF),
        );
        assert_eq!(shift_bc5_snorm_dds(&mut file), Ok(true));
        assert_eq!(
            file,
            dds(
                b"DX10",
                DXGI_FORMAT_BC5_UNORM,
                &block(0x01, 0xFF, 0x80, 0x7F)
            )
        );
    }

    #[test]
    fn shifts_a_legacy_bc5s_dds_without_changing_indices_or_other_header_bytes() {
        let mut file = dds(b"BC5S", 0, &[]);
        file.truncate(LEGACY_DATA_OFFSET);
        file.extend_from_slice(&block(0x81, 0x7F, 0x00, 0xFF));
        let mut expected = file.clone();
        expected[FOURCC_OFFSET..FOURCC_OFFSET + 4].copy_from_slice(b"ATI2");
        expected[LEGACY_DATA_OFFSET..].copy_from_slice(&block(0x01, 0xFF, 0x80, 0x7F));
        assert_eq!(shift_bc5_snorm_dds(&mut file), Ok(true));
        assert_eq!(file, expected);
        assert_eq!(shift_bc5_snorm_dds(&mut file), Ok(false));
    }

    #[test]
    fn leaves_every_other_file_alone() {
        let data = block(1, 2, 3, 4);
        for mut file in [
            dds(b"DX10", 98, &data),
            dds(b"DX10", DXGI_FORMAT_BC5_UNORM, &data),
            dds(b"ATI2", 0, &data),
            b"DDS ".to_vec(),
        ] {
            let before = file.clone();
            assert_eq!(shift_bc5_snorm_dds(&mut file), Ok(false));
            assert_eq!(file, before);
        }

        let mut partial = dds(b"DX10", DXGI_FORMAT_BC5_SNORM, &[0u8; 20]);
        let before = partial.clone();
        assert_eq!(
            shift_bc5_snorm_dds(&mut partial),
            Err(Bc5ShiftError::NotWholeBlocks { len: 20 })
        );
        assert_eq!(partial, before);
    }

    fn signed_normal_fixture(mips: bool) -> (Vec<u8>, Vec<u8>) {
        let pixels = [64, 192, 0, 255].repeat(8 * 8);
        let chain = if mips {
            directxtex_native::rgba8_box_mip_chain(8, 8, &pixels).unwrap()
        } else {
            vec![(8, 8, pixels)]
        };
        let unsigned =
            directxtex_native::encode_dds_from_rgba8_chain(&chain, "BC5_UNORM", false, None)
                .unwrap();
        let mut signed = unsigned.clone();
        let offset = if &signed[84..88] == b"DX10" {
            signed[128..132].copy_from_slice(&DXGI_FORMAT_BC5_SNORM.to_le_bytes());
            DX10_DATA_OFFSET
        } else {
            signed[84..88].copy_from_slice(b"BC5S");
            LEGACY_DATA_OFFSET
        };
        bc5_snorm_to_unorm_in_place(&mut signed[offset..]).unwrap();
        (signed, unsigned)
    }

    fn normal_request(
        input: std::path::PathBuf,
        output: std::path::PathBuf,
    ) -> TextureSetPathRequest {
        TextureSetPathRequest {
            source_game: "fo76".into(),
            target_game: "fo4".into(),
            inputs: vec![TexturePathInput {
                role: "normal".into(),
                path: input,
            }],
            outputs: vec![TexturePathOutput {
                role: "normal".into(),
                path: output,
                format: "BC5_UNORM".into(),
            }],
            params: Default::default(),
            use_gpu: false,
            gpu_min_pixels: u32::MAX,
            parallel_compression: false,
        }
    }

    #[test]
    fn path_converter_preserves_signed_normal_indices_and_authored_mips() {
        let dir = std::env::temp_dir().join(format!("b21_bc5_full_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source.dds");
        let output = dir.join("converted.dds");
        let (signed, unsigned) = signed_normal_fixture(true);
        std::fs::write(&input, &signed).unwrap();
        assert_eq!(
            directxtex_native::read_dds_probe(&input)
                .unwrap()
                .dxgi_format,
            84
        );
        let result =
            convert_texture_set_paths(normal_request(input.clone(), output.clone())).unwrap();
        assert_eq!(result.converted.len(), 1);
        assert_eq!(std::fs::read(output).unwrap(), unsigned);
        assert_eq!(std::fs::read(input).unwrap(), signed);
    }

    #[test]
    fn path_converter_generates_missing_mips_after_signed_normal_conversion() {
        let dir = std::env::temp_dir().join(format!("b21_bc5_mips_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source.dds");
        let output = dir.join("converted.dds");
        let (signed, _) = signed_normal_fixture(false);
        std::fs::write(&input, signed).unwrap();
        convert_texture_set_paths(normal_request(input, output.clone())).unwrap();
        let probe = directxtex_native::read_dds_probe(&output).unwrap();
        assert_eq!(probe.dxgi_format, 83);
        assert_eq!(probe.mip_levels, 4);
        let image = directxtex_native::read_dds_rgba_image(&output).unwrap();
        for pixel in image.rgba.chunks_exact(4) {
            assert!((i32::from(pixel[0]) - 64).abs() <= 2);
            assert!((i32::from(pixel[1]) - 192).abs() <= 2);
        }
    }

    #[test]
    fn path_converter_rejects_a_truncated_signed_normal_before_writing() {
        let dir = std::env::temp_dir().join(format!("b21_bc5_truncated_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source.dds");
        let output = dir.join("must_not_exist.dds");
        let (mut signed, _) = signed_normal_fixture(true);
        signed.truncate(signed.len() - 16);
        std::fs::write(&input, signed).unwrap();
        let error = convert_texture_set_paths(normal_request(input, output.clone())).unwrap_err();
        assert!(
            error.to_string().contains("payload length mismatch"),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    fn path_converter_rejects_non_2d_dx10_normal_headers_before_writing() {
        let dir = std::env::temp_dir().join(format!("b21_bc5_dimensions_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (legacy, _) = signed_normal_fixture(true);
        assert_eq!(&legacy[84..88], b"BC5S");
        let mut dx10 = legacy[..128].to_vec();
        dx10[84..88].copy_from_slice(b"DX10");
        for word in [84_u32, 3, 0, 1, 0] {
            dx10.extend_from_slice(&word.to_le_bytes());
        }
        dx10.extend_from_slice(&legacy[128..]);
        let input = dir.join("source.dds");
        for (index, (dimension, array_size, caps2)) in [
            (2_u32, 1_u32, 0_u32),
            (4, 1, 0),
            (0, 1, 0),
            (3, 0, 0),
            (3, 1, 0x400),
        ]
        .into_iter()
        .enumerate()
        {
            let mut invalid = dx10.clone();
            invalid[132..136].copy_from_slice(&dimension.to_le_bytes());
            invalid[140..144].copy_from_slice(&array_size.to_le_bytes());
            invalid[112..116].copy_from_slice(&caps2.to_le_bytes());
            std::fs::write(&input, invalid).unwrap();
            let output = dir.join(format!("must_not_exist{index}.dds"));
            let error = convert_texture_set_paths(normal_request(input.clone(), output.clone()))
                .unwrap_err();
            assert!(error.to_string().contains("must be a 2D image"), "{error}");
            assert!(!output.exists());
        }
        std::fs::write(&input, &dx10).unwrap();
        let output = dir.join("valid.dds");
        convert_texture_set_paths(normal_request(input, output.clone())).unwrap();
        shift_bc5_snorm_dds(&mut dx10).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), dx10);
    }

    #[test]
    #[ignore = "requires extracted Starfield Stinger normal"]
    fn real_stinger_signed_normal_uses_the_path_converter_and_keeps_all_mips() {
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extracted/starfield/Textures/Weapons/stinger_pistol/stinger_pistol_normal.dds");
        let output = std::env::temp_dir().join("b21_sf_stinger_normal_converted.dds");
        let source = std::fs::read(&input).unwrap();
        assert_eq!(&source[84..88], b"BC5S");
        let source_probe = directxtex_native::read_dds_probe(&input).unwrap();
        assert_eq!(source_probe.dxgi_format, 84);
        let mut expected = source.clone();
        assert!(shift_bc5_snorm_dds(&mut expected).unwrap());
        convert_texture_set_paths(normal_request(input.clone(), output.clone())).unwrap();
        assert_eq!(std::fs::read(&input).unwrap(), source);
        assert_eq!(std::fs::read(&output).unwrap(), expected);
        let target_probe = directxtex_native::read_dds_probe(&output).unwrap();
        assert_eq!(target_probe.dxgi_format, 83);
        assert_eq!(target_probe.mip_levels, source_probe.mip_levels);
        let image = directxtex_native::read_dds_rgba_image(&output).unwrap();
        let negative_components = image
            .rgba
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1]])
            .filter(|value| *value < 128)
            .count();
        assert!(negative_components > 0);
        std::fs::write(
            std::env::temp_dir().join("b21_sf_stinger_normal_receipt.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "source":input,"output":output,"source_format":84,"target_format":83,
                "width":source_probe.width,"height":source_probe.height,
                "mips":target_probe.mip_levels,"bytes":expected.len(),
                "negative_normal_components":negative_components,
                "indices_and_mips_preserved":true,"source_unchanged":true,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
