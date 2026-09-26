use crate::{DXGI_FORMAT, bc_level_size, dds_dx10_header, dxtex_compressed_payload, ispc_bc7_payload, read_dds_mips_rgba8};

fn rmse(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len(), "buffer size mismatch");
    let sum: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = f64::from(*x) - f64::from(*y);
            d * d
        })
        .sum();
    (sum / a.len() as f64).sqrt()
}

/// Encoded output quality: write the DDS bytes, decode mip0, RMSE vs input.
fn decode_mip0_rmse(dds_bytes: &[u8], input_rgba: &[u8], tag: &str) -> f64 {
    let tmp = std::env::temp_dir().join(format!("encode_bench_{tag}.dds"));
    std::fs::write(&tmp, dds_bytes).expect("write bench dds");
    let decoded = read_dds_mips_rgba8(&tmp).expect("decode bench dds");
    let _ = std::fs::remove_file(&tmp);
    rmse(&decoded.mips[0].2, input_rgba)
}

/// Non-ignored quality guard: on a synthetic image the ISPC production
/// profile must decode at least as close to the input as the DirectXTex
/// quick encoder it replaced (small tolerance for mode-choice noise).
#[test]
fn ispc_bc7_quality_matches_or_beats_dxtex_quick() {
    let (w, h) = (64usize, 64usize);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            rgba[i] = (x * 4) as u8;
            rgba[i + 1] = (y * 4) as u8;
            rgba[i + 2] = ((x ^ y) * 4) as u8;
            rgba[i + 3] = 255;
        }
    }
    let assemble = |payload: &[u8]| {
        let mut out = dds_dx10_header(
            w as u32,
            h as u32,
            bc_level_size(w, h, 2).unwrap() as u32,
            DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM,
            1,
        );
        out.extend_from_slice(payload);
        out
    };
    let dxt =
        dxtex_compressed_payload(w, h, &rgba, DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM, false).unwrap();
    let ispc = ispc_bc7_payload(w, h, &rgba, false).unwrap();
    assert_eq!(dxt.len(), ispc.len(), "payload sizes must match");
    let dxt_rmse = decode_mip0_rmse(&assemble(&dxt), &rgba, "quality_dxt");
    let ispc_rmse = decode_mip0_rmse(&assemble(&ispc), &rgba, "quality_ispc");
    eprintln!("quality: dxtex_quick rmse={dxt_rmse:.4} ispc_basic rmse={ispc_rmse:.4}");
    assert!(
        ispc_rmse <= dxt_rmse + 0.25,
        "ispc rmse {ispc_rmse:.4} materially worse than dxtex quick {dxt_rmse:.4}"
    );
}
