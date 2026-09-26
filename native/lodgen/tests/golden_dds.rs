// Terrain .dds composite: tile sizes, baked diffuse/normal content, coverage.

#[test]
fn our_composite_tile_matches_expected_size_for_lod4() {
    // Drive composite_quad for a synthetic 4x4 world at L4 and verify 256x256.
    // Cells carry an LTEX layer so the per-level (256) size applies — a quad with
    // NO layer is a default tile sized at default_diffuse_size (128); see
    // `terrain::textures::tests::default_cell_quad_uses_default_dds_size`.
    let world = lodgen_native::input::WorldspaceInput::from_cells(
        "W",
        (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| lodgen_native::input::CellInput {
                x,
                y,
                heights: vec![0.0f32; 33 * 33],
                vertex_colors: vec![[255u8, 255, 255]; 33 * 33],
                layers: vec![lodgen_native::input::LayerTexture {
                    diffuse: "textures/landscape/dirt01.dds".into(),
                    normal: String::new(),
                    quadrant: 0,
                    alpha: vec![1.0; 17 * 17],
                }],
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect(),
    );
    let s = lodgen_native::settings::LodSettings::fo4_default();
    let quad = lodgen_native::descriptors::quads_for(&world, 4, &s)
        .into_iter()
        .find(|q| q.x == 0 && q.y == 0)
        .unwrap();
    let paths = lodgen_native::progress::LodPaths {
        data_dirs: vec![std::path::PathBuf::from(".")],
        output_dir: std::env::temp_dir().join("lodgen_golden_dds_test"),
        source_data_dir: None,
    };

    let tile = lodgen_native::terrain::textures::composite_quad(&world, &quad, &s, &paths).unwrap();
    // L4 diffuse_size default = 256 (real golden size, fo4_default).
    assert_eq!(tile.width, 256);
    assert_eq!(tile.height, 256);
    assert_eq!(tile.diffuse_rgba.len(), 256 * 256 * 4);
}

/// Per-channel (mean, std) over an RGBA8 buffer (alpha ignored).
fn channel_stats(rgba: &[u8]) -> ([f64; 3], [f64; 3]) {
    let n = (rgba.len() / 4) as f64;
    let mut sum = [0.0f64; 3];
    for px in rgba.chunks_exact(4) {
        for c in 0..3 {
            sum[c] += px[c] as f64;
        }
    }
    let mean = [sum[0] / n, sum[1] / n, sum[2] / n];
    let mut var = [0.0f64; 3];
    for px in rgba.chunks_exact(4) {
        for c in 0..3 {
            let d = px[c] as f64 - mean[c];
            var[c] += d * d;
        }
    }
    let std = [
        (var[0] / n).sqrt(),
        (var[1] / n).sqrt(),
        (var[2] / n).sqrt(),
    ];
    (mean, std)
}

/// Pixel-CONTENT regression for the two bugs: (1) a resolvable, prefix-less
/// landscape diffuse path must produce a textured (non-uniform) diffuse — NOT
/// the blank ~grey fallback; (2) sloped cell heights must produce a real,
/// green-dominant `_msn` — NOT a flat constant. An all-grey diffuse (σ≈0) or a
/// σ=0 `_msn` MUST fail here.
#[test]
fn baked_tile_has_textured_diffuse_and_sloped_green_normal() {
    // Write a synthetic, high-variance source diffuse under <tmp>/textures/... .
    // The layer references it PREFIX-LESS ("landscape/synth_d.dds") to exercise
    // the FO4 `Textures\`-prefix probing in load_texture (Fix A).
    let root = std::env::temp_dir().join("lodgen_pixel_content_test");
    let tex_dir = root.join("textures").join("landscape");
    std::fs::create_dir_all(&tex_dir).unwrap();
    let src = tex_dir.join("synth_d.dds");
    let (sw, sh) = (32u32, 32u32);
    let mut src_rgba = vec![0u8; (sw * sh * 4) as usize];
    for y in 0..sh {
        for x in 0..sw {
            let i = ((y * sw + x) * 4) as usize;
            // strong, non-uniform terrain-ish color gradient
            src_rgba[i] = (x * 8) as u8; // R
            src_rgba[i + 1] = (y * 6) as u8; // G
            src_rgba[i + 2] = (x + y) as u8; // B
            src_rgba[i + 3] = 255;
        }
    }
    directxtex_native::write_dds_rgba_image(&src, sw, sh, &src_rgba, "R8G8B8A8_UNORM", false)
        .expect("write synthetic source dds");

    // 4x4 world; each cell has a non-linear (paraboloid) heightfield so the
    // surface normal VARIES across the tile (a linear ramp would be uniform).
    let cell_heights: Vec<f32> = (0..33 * 33)
        .map(|i| {
            let c = (i % 33) as f32;
            let r = (i / 33) as f32;
            (c * c + r * r) * 2.0
        })
        .collect();
    let world = lodgen_native::input::WorldspaceInput::from_cells(
        "W",
        (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| lodgen_native::input::CellInput {
                x,
                y,
                heights: cell_heights.clone(),
                vertex_colors: vec![[255u8, 255, 255]; 33 * 33],
                // base layer (empty alpha => fully opaque over the whole cell).
                layers: vec![lodgen_native::input::LayerTexture {
                    diffuse: "landscape/synth_d.dds".into(),
                    normal: String::new(),
                    quadrant: 0,
                    alpha: Vec::new(),
                }],
                hidden_quadrants: [false; 4],
                water_height: f32::MIN,
            })
            .collect(),
    );
    let s = lodgen_native::settings::LodSettings::fo4_default();
    let quad = lodgen_native::descriptors::quads_for(&world, 4, &s)
        .into_iter()
        .find(|q| q.x == 0 && q.y == 0)
        .unwrap();
    let paths = lodgen_native::progress::LodPaths {
        data_dirs: vec![root.clone()],
        output_dir: std::env::temp_dir().join("lodgen_pixel_content_out"),
        source_data_dir: None,
    };

    let tile = lodgen_native::terrain::textures::composite_quad(&world, &quad, &s, &paths).unwrap();

    // (1) Diffuse: textured, not the blank ~grey fallback. The blank bug produced
    //     a uniform fill; a resolved source gives clear variance.
    let (dmean, dstd) = channel_stats(&tile.diffuse_rgba);
    assert!(
        dstd[0] > 15.0 || dstd[1] > 15.0 || dstd[2] > 15.0,
        "diffuse must be textured (σ≫0), got mean={dmean:?} std={dstd:?} (blank-fallback regression)"
    );

    // (2) `_msn`: real, green-dominant model-space normal. The flat-normal bug
    //     produced σ=0; a blue-dominant tangent normal would fail green-dominance.
    let (nmean, nstd) = channel_stats(&tile.normal_rgba);
    assert!(
        nmean[1] > nmean[0] && nmean[1] > nmean[2],
        "_msn must be GREEN-dominant (world-up in green), got mean={nmean:?}"
    );
    assert!(
        nstd[0] > 3.0 || nstd[2] > 3.0,
        "_msn must vary with slope (σ≫0 on R/B), got std={nstd:?} (flat-normal regression)"
    );
    // Green (up) stays high; horizontal channels center near 128.
    assert!(
        nmean[1] > 200.0,
        "_msn green (up) should be high, got {}",
        nmean[1]
    );

    let _ = std::fs::remove_dir_all(&root);
}

