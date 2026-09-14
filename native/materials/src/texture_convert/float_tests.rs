use super::*;

fn fixture_pixels(width: usize, height: usize, seed: usize) -> Vec<f32> {
    (0..width * height * 4)
        .map(|i| (((i * 37 + seed * 53) % 347) as f32 - 30.0) / 255.0)
        .collect()
}

fn fixture_outputs() -> Vec<u8> {
    let mut bytes = Vec::new();
    for (width, height, other_width, other_height) in [(1, 1, 1, 1), (3, 2, 2, 3), (8, 4, 4, 2)] {
        let d = f32_vec_to_bytes(&fixture_pixels(width, height, 1));
        let r = f32_vec_to_bytes(&fixture_pixels(other_width, other_height, 2));
        let l = f32_vec_to_bytes(&fixture_pixels(other_width, other_height, 3));
        for preserve_glow in [false, true] {
            let mut params: TextureConversionParams =
                TextureConversionParamsPayload::default().into();
            params.preserve_lighting_rgb_for_glow = preserve_glow;
            let out = fo76_bundle_to_fo4_buffers(
                &d,
                &r,
                &l,
                width,
                height,
                other_width,
                other_height,
                other_width,
                other_height,
                params,
                true,
            )
            .unwrap();
            bytes.extend(f32_vec_to_bytes(&out.diffuse));
            bytes.extend(f32_vec_to_bytes(&out.specgloss));
            bytes.extend(f32_vec_to_bytes(out.glow.as_ref().unwrap()));
            bytes.extend(f32_vec_to_bytes(
                &fo76_lighting_to_fo4_glow_buffer(&d, width, height, preserve_glow).unwrap(),
            ));
        }
        bytes.extend(f32_vec_to_bytes(
            &fo76_normal_to_fo4_buffer(&d, width, height).unwrap(),
        ));
        bytes.extend(f32_vec_to_bytes(
            &fo76_normalized_normal_to_fo4_buffer(&d, width, height).unwrap(),
        ));
        bytes.extend(f32_vec_to_bytes(
            &passthrough_rgba_buffer(&d, width, height).unwrap(),
        ));
        bytes.extend(f32_vec_to_bytes(
            &fo76_reflectivity_lighting_to_fo4_specgloss_buffers(
                &d,
                &l,
                width,
                height,
                other_width,
                other_height,
            )
            .unwrap(),
        ));
        for mask in [None, Some(r.as_slice())] {
            let out = gamebryo_normal_envmask_to_fo4_specgloss_buffers(
                &d,
                mask,
                width,
                height,
                other_width,
                other_height,
                GamebryoSpecParams::default(),
            )
            .unwrap();
            bytes.extend(f32_vec_to_bytes(&out.normal));
            bytes.extend(f32_vec_to_bytes(&out.specgloss));
            let out = starfield_pbr_to_fo4_buffers(
                &d,
                &r,
                &l,
                mask,
                width,
                height,
                other_width,
                other_height,
                other_width,
                other_height,
                other_width,
                other_height,
                TextureConversionParamsPayload::default().into(),
            )
            .unwrap();
            bytes.extend(f32_vec_to_bytes(&out.diffuse));
            bytes.extend(f32_vec_to_bytes(&out.specgloss));
        }
    }
    bytes
}

#[test]
fn float_kernels_match_pre_optimization_bytes() {
    assert_eq!(fixture_outputs(), include_bytes!("float_kernels.bin"));
}

#[test]
fn typed_pixels_reject_invalid_lengths_and_dimensions() {
    assert!(fo76_normalized_normal_to_fo4_pixels(&[0.0; 3], 1, 1).is_err());
    assert!(passthrough_rgba_pixels(&[], 0, 1).is_err());
    assert!(fo76_lighting_to_fo4_glow_pixels(&[], usize::MAX, 2, false).is_err());
}

#[test]
#[ignore = "Paired native-float versus byte-wrapper benchmark"]
fn benchmark_float_kernels() {
    use std::time::Instant;
    let mut rows = Vec::new();
    for side in [512, 1024] {
        let d = fixture_pixels(side, side, 1);
        let r = fixture_pixels(side / 2, side / 2, 2);
        let l = fixture_pixels(side / 2, side / 2, 3);
        let mut legacy_seconds = 0.0;
        let mut typed_seconds = 0.0;
        for iteration in 0..8 {
            let mut results = Vec::new();
            for typed in if iteration % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let started = Instant::now();
                let result = if typed {
                    fo76_bundle_to_fo4_pixels(
                        &d,
                        &r,
                        &l,
                        side,
                        side,
                        side / 2,
                        side / 2,
                        side / 2,
                        side / 2,
                        TextureConversionParamsPayload::default().into(),
                        true,
                    )
                } else {
                    fo76_bundle_to_fo4_buffers(
                        &f32_vec_to_bytes(&d),
                        &f32_vec_to_bytes(&r),
                        &f32_vec_to_bytes(&l),
                        side,
                        side,
                        side / 2,
                        side / 2,
                        side / 2,
                        side / 2,
                        TextureConversionParamsPayload::default().into(),
                        true,
                    )
                }
                .unwrap();
                let seconds = started.elapsed().as_secs_f64();
                if typed {
                    typed_seconds += seconds;
                } else {
                    legacy_seconds += seconds;
                }
                results.push(result);
            }
            for values in [
                (&results[0].diffuse, &results[1].diffuse),
                (&results[0].specgloss, &results[1].specgloss),
                (
                    results[0].glow.as_ref().unwrap(),
                    results[1].glow.as_ref().unwrap(),
                ),
            ] {
                assert!(
                    values
                        .0
                        .iter()
                        .zip(values.1)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                );
            }
        }
        let row = format!(
            "{{\"side\":{side},\"iterations\":8,\"byte_wrapper_seconds\":{legacy_seconds},\"native_float_seconds\":{typed_seconds}}}"
        );
        eprintln!("{row}");
        rows.push(row);
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .join("tmp/texture_optimization_20260909");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("float_benchmark.json"),
        format!("[{}]\n", rows.join(",\n")),
    )
    .unwrap();
}
