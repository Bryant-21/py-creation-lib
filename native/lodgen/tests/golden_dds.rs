// .dds visual-diff gate: golden xLODGen terrain tiles (dimensions, format, mips) and
// our composite tile sizes. Tests SKIP (with a notice) if the corpus is absent.
//
// Ground truth (tmp/xlodgen/Textures/Terrain/DLC03FarHarbor):
//   - Terrain tiles are 256x256 BC1/DXT1 (dxgi 71); 128x128 BC1 for default/empty cells.
//   - Only L4 diffuse has mips (9 @256, 8 @128). L8/L16/L32 diffuse and all `_msn`
//     normals are single-mip (mip_levels == 1).

use std::path::PathBuf;

fn corpus(rel: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join(rel);
    let p = p.canonicalize().unwrap_or(p);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("SKIP: missing {}", p.display());
        None
    }
}

// BC1_UNORM == dxgi 71 (golden corpus terrain tiles are DXT1/BC1).
const DXGI_BC1: u32 = 71;

#[test]
fn golden_l16_diffuse_is_256_bc1_single_mip() {
    let Some(path) =
        corpus("tmp/xlodgen/Textures/Terrain/DLC03FarHarbor/dlc03farharbor.16.-25.-11.dds")
    else {
        return;
    };
    let probe = directxtex_native::read_dds_probe(&path).expect("read dds probe");
    // Real golden size is 256x256 (NOT 64px/cell * 16 = 1024).
    assert_eq!(probe.width, 256, "L16 diffuse width");
    assert_eq!(probe.height, 256, "L16 diffuse height");
    assert_eq!(probe.dxgi_format, DXGI_BC1, "L16 diffuse format");
    // L8/L16/L32 diffuse carry no mips.
    assert_eq!(probe.mip_levels, 1, "L16 diffuse mip_levels");
}

#[test]
fn golden_l16_msn_is_256_bc1_single_mip() {
    let Some(path) =
        corpus("tmp/xlodgen/Textures/Terrain/DLC03FarHarbor/dlc03farharbor.16.-25.-11_msn.dds")
    else {
        return;
    };
    let probe = directxtex_native::read_dds_probe(&path).expect("read _msn probe");
    assert_eq!(probe.width, 256, "L16 _msn width");
    assert_eq!(probe.height, 256, "L16 _msn height");
    assert_eq!(probe.dxgi_format, DXGI_BC1, "L16 _msn format");
    assert_eq!(probe.mip_levels, 1, "L16 _msn mip_levels");
}

#[test]
fn golden_l4_diffuse_is_mipped_but_msn_is_not() {
    // L4 is the ONLY level whose diffuse carries a full mip chain (9 mips @256).
    let Some(diff) =
        corpus("tmp/xlodgen/Textures/Terrain/DLC03FarHarbor/dlc03farharbor.4.-1.-11.dds")
    else {
        return;
    };
    let dp = directxtex_native::read_dds_probe(&diff).expect("read L4 diffuse probe");
    assert_eq!(dp.width, 256, "L4 diffuse width");
    assert_eq!(dp.height, 256, "L4 diffuse height");
    assert_eq!(dp.dxgi_format, DXGI_BC1, "L4 diffuse format");
    // full chain for 256: floor(log2(256))+1 = 9.
    assert_eq!(dp.mip_levels, 9, "L4 diffuse mip_levels (full chain)");
    assert_eq!(
        dp.mip_levels,
        directxtex_native::full_mip_count(256, 256),
        "L4 diffuse should be a full mip chain"
    );

    let Some(msn) =
        corpus("tmp/xlodgen/Textures/Terrain/DLC03FarHarbor/dlc03farharbor.4.-1.-11_msn.dds")
    else {
        return;
    };
    let np = directxtex_native::read_dds_probe(&msn).expect("read L4 _msn probe");
    assert_eq!(np.width, 256, "L4 _msn width");
    // _msn is single-mip even at L4.
    assert_eq!(np.mip_levels, 1, "L4 _msn mip_levels");
}

#[test]
fn golden_default_cell_tile_is_128() {
    // Cells with no LTEX layers get the smaller "default" tile (128x128).
    let Some(path) =
        corpus("tmp/xlodgen/Textures/Terrain/DLC03FarHarbor/dlc03farharbor.16.-41.-27.dds")
    else {
        return;
    };
    let probe = directxtex_native::read_dds_probe(&path).expect("read default tile probe");
    assert_eq!(probe.width, 128, "default tile width");
    assert_eq!(probe.height, 128, "default tile height");
    assert_eq!(probe.dxgi_format, DXGI_BC1, "default tile format");
}

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

/// Fraction of near-black pixels (luminance < `thresh`) in an RGBA8 buffer.
fn near_black_fraction(rgba: &[u8], thresh: u32) -> f64 {
    let n = (rgba.len() / 4).max(1);
    let k = rgba
        .chunks_exact(4)
        .filter(|p| (p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 < thresh)
        .count();
    k as f64 / n as f64
}

/// The xLODGen golden terrain diffuse is a CONTINUOUS, fully-covered surface:
/// near-zero black pixels. This documents the coverage target our composite must
/// match (skips if the corpus is absent).
#[test]
fn golden_terrain_diffuse_is_continuous_no_black() {
    let Some(path) =
        corpus("tmp/xlodgen/Textures/Terrain/DLC03FarHarbor/dlc03farharbor.16.-25.-11.dds")
    else {
        return;
    };
    // decode via the BC1 reader bundled in directxtex_native.
    let img = directxtex_native::read_dds_rgba_image(&path).expect("decode golden diffuse");
    let nb = near_black_fraction(&img.rgba, 16);
    assert!(
        nb < 0.01,
        "golden terrain diffuse must be continuous (near-black < 1%), got {:.1}%",
        nb * 100.0
    );
}

/// A cell whose only layer has sparse, quadrant-limited alpha and dark vertex colors
/// must still composite to a fully covered tile: the cell's first resolvable layer
/// texture is painted as the opaque full-cell base. A grey-128 fallback times dark
/// VCLR turns the uncovered ~3/4 of each cell near-black, which the near-black
/// threshold catches (a variance check alone does not).
#[test]
fn composite_diffuse_is_fully_covered_no_black_gaps() {
    let root = std::env::temp_dir().join("lodgen_coverage_gap_test");
    let tex_dir = root.join("textures").join("landscape");
    std::fs::create_dir_all(&tex_dir).unwrap();
    // Bright source texture (every channel >= 200) so covered pixels survive the
    // dark VCLR multiply (>16) while a grey-128 fallback would not (128*0.094≈12).
    let src = tex_dir.join("bright_d.dds");
    let (sw, sh) = (32u32, 32u32);
    let mut src_rgba = vec![0u8; (sw * sh * 4) as usize];
    for y in 0..sh {
        for x in 0..sw {
            let i = ((y * sw + x) * 4) as usize;
            src_rgba[i] = 200 + (x as u8); // R 200..231
            src_rgba[i + 1] = 200 + (y as u8); // G 200..231
            src_rgba[i + 2] = 210; // B
            src_rgba[i + 3] = 255;
        }
    }
    directxtex_native::write_dds_rgba_image(&src, sw, sh, &src_rgba, "R8G8B8A8_UNORM", false)
        .expect("write bright source dds");

    // Sparse per-quadrant alpha: only the quadrant interior is opaque; the cell
    // edges and other quadrants rely on the full-cell base.
    let mut alpha = vec![0.0f32; 17 * 17];
    for r in 4..13 {
        for c in 4..13 {
            alpha[r * 17 + c] = 1.0;
        }
    }
    let world = lodgen_native::input::WorldspaceInput::from_cells(
        "W",
        (0..4)
            .flat_map(|y| (0..4).map(move |x| (x, y)))
            .map(|(x, y)| lodgen_native::input::CellInput {
                x,
                y,
                heights: vec![0.0f32; 33 * 33],
                // dark vertex colors so any grey-fallback gap goes BLACK.
                vertex_colors: vec![[24u8, 24, 24]; 33 * 33],
                layers: vec![lodgen_native::input::LayerTexture {
                    diffuse: "landscape/bright_d.dds".into(),
                    normal: String::new(),
                    quadrant: 0,
                    alpha: alpha.clone(),
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
        output_dir: std::env::temp_dir().join("lodgen_coverage_gap_out"),
        source_data_dir: None,
    };

    let tile = lodgen_native::terrain::textures::composite_quad(&world, &quad, &s, &paths).unwrap();
    let nb = near_black_fraction(&tile.diffuse_rgba, 16);
    assert!(
        nb < 0.01,
        "diffuse must be fully covered (near-black < 1%), got {:.1}% (black-gap regression)",
        nb * 100.0
    );
    // And no pixel should be the neutral grey fallback (128,128,128 before VCLR →
    // ~12 after): the base must be the real land texture everywhere.
    let grey_after_vclr = tile
        .diffuse_rgba
        .chunks_exact(4)
        .filter(|p| p[0] == p[1] && p[1] == p[2] && p[0] < 14)
        .count();
    assert_eq!(
        grey_after_vclr, 0,
        "no pixel may fall through to the grey base fallback"
    );

    let _ = std::fs::remove_dir_all(&root);
}
