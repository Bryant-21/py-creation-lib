//! Env-gated BC7 encoder A/B benchmark on real corpus textures. `--ignored` only.
//! Corpus root: MODBOX_FO76_EXTRACTED else <repo>/extracted/fo76; skips if absent.
//! Measures the production per-worker encode shape (full mip chain, single
//! thread, no GPU) and reports Mpx/s + decode-RMSE-vs-input per variant.

use std::path::{Path, PathBuf};
use std::time::Instant;

use intel_tex_2::bc7;

use crate::{
    DXGI_FORMAT, bc_level_size, convert_unorm_texels_to_srgb, dds_base_rgba, dds_dx10_header,
    dxtex_compressed_payload, ispc_bc, ispc_bc7_payload, read_dds_mips_rgba8, rgba_mip_chain,
};

fn fo76_textures_dir() -> Option<PathBuf> {
    let root = std::env::var("MODBOX_FO76_EXTRACTED")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .expect("crate lives at repo/py_creation_lib/native/directxtex")
                .join("extracted")
                .join("fo76")
        });
    let dir = root.join("Textures");
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!("skip: FO76 extracted dir absent at {}", dir.display());
        None
    }
}

fn walk_sorted(dir: &Path, out: &mut Vec<PathBuf>, wanted: usize) {
    if out.len() >= wanted {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in &entries {
        if out.len() >= wanted {
            return;
        }
        if path.is_file()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().ends_with("_d.dds"))
        {
            out.push(path.clone());
        }
    }
    for path in &entries {
        if out.len() >= wanted {
            return;
        }
        if path.is_dir() {
            walk_sorted(path, out, wanted);
        }
    }
}

struct Sample {
    name: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn corpus_sample(large: usize, small: usize) -> Vec<Sample> {
    let Some(dir) = fo76_textures_dir() else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    walk_sorted(&dir, &mut candidates, (large + small) * 20);
    let mut larges = Vec::new();
    let mut smalls = Vec::new();
    for path in candidates {
        if larges.len() >= large && smalls.len() >= small {
            break;
        }
        let Ok((w, h, rgba, _)) = dds_base_rgba(&path) else {
            continue;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sample = Sample {
            name,
            width: w,
            height: h,
            rgba,
        };
        if w.min(h) >= 1024 && larges.len() < large {
            larges.push(sample);
        } else if (128..512).contains(&w.min(h)) && smalls.len() < small {
            smalls.push(sample);
        }
    }
    larges.extend(smalls);
    larges
}

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

fn chain_pixels(width: u32, height: u32) -> u64 {
    let (mut w, mut h) = (u64::from(width), u64::from(height));
    let mut px = 0u64;
    loop {
        px += w * h;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    px
}

type PayloadFn = dyn Fn(usize, usize, &[u8]) -> Result<Vec<u8>, String> + Sync;

/// Full-DDS assembly around a per-level BC7 payload encoder — mirrors
/// `encode_compressed_dds` for BC7_UNORM_SRGB with the payload
/// encoder swapped per variant.
fn dds_via(width: u32, height: u32, rgba: &[u8], payload: &PayloadFn) -> Result<Vec<u8>, String> {
    let chain = rgba_mip_chain(width as usize, height as usize, rgba, true)?;
    let linear_size = bc_level_size(width as usize, height as usize, 2)?;
    let mut out = dds_dx10_header(
        width,
        height,
        linear_size as u32,
        DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB,
        chain.len() as u32,
    );
    for (w, h, level) in &chain {
        out.extend_from_slice(&payload(*w, *h, level)?);
    }
    Ok(out)
}

fn ispc_payload_with(
    profile: fn(&[u8]) -> bc7::EncodeSettings,
) -> impl Fn(usize, usize, &[u8]) -> Result<Vec<u8>, String> + Sync {
    move |w, h, rgba| {
        let converted = convert_unorm_texels_to_srgb(w, h, rgba)?;
        let settings = profile(&converted);
        ispc_bc::bc7_blocks_from_rgba(w, h, &converted, &settings)
    }
}

fn fast_profile(rgba: &[u8]) -> bc7::EncodeSettings {
    if rgba.chunks_exact(4).all(|px| px[3] == 0xFF) {
        bc7::opaque_fast_settings()
    } else {
        bc7::alpha_fast_settings()
    }
}

fn very_fast_profile(rgba: &[u8]) -> bc7::EncodeSettings {
    if rgba.chunks_exact(4).all(|px| px[3] == 0xFF) {
        bc7::opaque_very_fast_settings()
    } else {
        bc7::alpha_very_fast_settings()
    }
}

#[test]
#[ignore]
fn bench_bc7_encoders() {
    let samples = corpus_sample(24, 8);
    if samples.is_empty() {
        return;
    }
    let total_px: u64 = samples
        .iter()
        .map(|s| chain_pixels(s.width, s.height))
        .sum();
    eprintln!(
        "corpus sample: {} textures, {:.1} Mpx incl. mips",
        samples.len(),
        total_px as f64 / 1e6
    );
    for s in &samples {
        eprintln!("  {}x{} {}", s.width, s.height, s.name);
    }

    let variants: Vec<(&'static str, Box<PayloadFn>)> = vec![
        (
            "dxtex_cpu_quick",
            Box::new(|w, h, rgba: &[u8]| {
                dxtex_compressed_payload(w, h, rgba, DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB, false)
            }),
        ),
        (
            "ispc_basic_prod",
            Box::new(|w, h, rgba: &[u8]| ispc_bc7_payload(w, h, rgba, true)),
        ),
        ("ispc_fast", Box::new(ispc_payload_with(fast_profile))),
        (
            "ispc_very_fast",
            Box::new(ispc_payload_with(very_fast_profile)),
        ),
    ];

    for (name, payload) in &variants {
        // Warmup on the smallest sample to exclude one-time init.
        let smallest = samples
            .iter()
            .min_by_key(|s| s.width * s.height)
            .expect("non-empty");
        let _ = dds_via(smallest.width, smallest.height, &smallest.rgba, payload);

        let started = Instant::now();
        let mut outputs = Vec::with_capacity(samples.len());
        for s in &samples {
            outputs.push(dds_via(s.width, s.height, &s.rgba, payload).expect("encode"));
        }
        let secs = started.elapsed().as_secs_f64();

        let mut worst_rmse = 0.0f64;
        let mut sum_rmse = 0.0f64;
        for (s, dds) in samples.iter().zip(&outputs) {
            let e = decode_mip0_rmse(dds, &s.rgba, name);
            sum_rmse += e;
            worst_rmse = worst_rmse.max(e);
        }
        eprintln!(
            "{name}: {:.1}s  {:.2} Mpx/s  {:.2} tex/s  rmse avg={:.3} worst={:.3}",
            secs,
            total_px as f64 / 1e6 / secs,
            samples.len() as f64 / secs,
            sum_rmse / samples.len() as f64,
            worst_rmse,
        );
    }

    // GPU chain-batch path for routing comparison (one warmup submission,
    // then the same samples through the production use_gpu=true branch).
    let smallest = samples
        .iter()
        .min_by_key(|s| s.width * s.height)
        .expect("non-empty");
    let warm = crate::encode_compressed_dds(
        smallest.width,
        smallest.height,
        &smallest.rgba,
        DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB,
        true,
        false,
        true,
    );
    if warm.is_err() {
        eprintln!("gpu_batch: unavailable on this machine, skipping");
        return;
    }
    let started = Instant::now();
    for s in &samples {
        crate::encode_compressed_dds(
            s.width,
            s.height,
            &s.rgba,
            DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB,
            true,
            false,
            true,
        )
        .expect("gpu encode");
    }
    let secs = started.elapsed().as_secs_f64();
    eprintln!(
        "gpu_batch: {:.1}s  {:.2} Mpx/s  {:.2} tex/s",
        secs,
        total_px as f64 / 1e6 / secs,
        samples.len() as f64 / secs,
    );
}

/// Corpus-scale A/B: ~2000 real diffuse textures, encoded with rayon
/// parallelism to mirror the production multi-worker wave. Decode time is
/// excluded (both variants encode the same in-memory batch).
#[test]
#[ignore]
fn bench_bc7_encoders_2k() {
    use rayon::prelude::*;

    let Some(dir) = fo76_textures_dir() else {
        return;
    };
    let wanted: usize = std::env::var("MODBOX_BENCH_TEXTURES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let mut paths = Vec::new();
    walk_sorted(&dir, &mut paths, wanted);
    eprintln!(
        "2k bench: {} textures, {} rayon threads",
        paths.len(),
        rayon::current_num_threads()
    );

    let variants: Vec<(&'static str, Box<PayloadFn>)> = vec![
        (
            "dxtex_cpu_quick",
            Box::new(|w, h, rgba: &[u8]| {
                dxtex_compressed_payload(w, h, rgba, DXGI_FORMAT::DXGI_FORMAT_BC7_UNORM_SRGB, false)
            }),
        ),
        (
            "ispc_basic_prod",
            Box::new(|w, h, rgba: &[u8]| ispc_bc7_payload(w, h, rgba, true)),
        ),
    ];

    let mut secs = vec![0.0f64; variants.len()];
    let mut rmse_sum = vec![0.0f64; variants.len()];
    let mut rmse_worst = vec![0.0f64; variants.len()];
    let mut rmse_n = vec![0u64; variants.len()];
    let mut total_px = 0u64;
    let mut encoded = 0u64;

    for batch in paths.chunks(128) {
        let samples: Vec<Sample> = batch
            .par_iter()
            .filter_map(|path| {
                let (w, h, rgba, _) = dds_base_rgba(path).ok()?;
                if w.min(h) < 16 {
                    return None;
                }
                Some(Sample {
                    name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    width: w,
                    height: h,
                    rgba,
                })
            })
            .collect();
        if samples.is_empty() {
            continue;
        }
        total_px += samples
            .iter()
            .map(|s| chain_pixels(s.width, s.height))
            .sum::<u64>();
        encoded += samples.len() as u64;

        for (i, (name, payload)) in variants.iter().enumerate() {
            let started = Instant::now();
            let outputs: Vec<Vec<u8>> = samples
                .par_iter()
                .map(|s| dds_via(s.width, s.height, &s.rgba, payload).expect("encode"))
                .collect();
            secs[i] += started.elapsed().as_secs_f64();
            for (j, (s, dds)) in samples.iter().zip(&outputs).enumerate() {
                if j % 64 == 0 {
                    let e = decode_mip0_rmse(dds, &s.rgba, name);
                    rmse_sum[i] += e;
                    rmse_worst[i] = rmse_worst[i].max(e);
                    rmse_n[i] += 1;
                }
            }
        }
        eprintln!(
            "  [{} tex] dxtex {:.1}s vs ispc {:.1}s",
            encoded, secs[0], secs[1]
        );
    }

    eprintln!(
        "2k bench totals: {} textures, {:.1} Mpx incl. mips",
        encoded,
        total_px as f64 / 1e6
    );
    for (i, (name, _)) in variants.iter().enumerate() {
        eprintln!(
            "{name}: {:.1}s  {:.2} Mpx/s  {:.2} tex/s  rmse avg={:.3} worst={:.3} (n={})",
            secs[i],
            total_px as f64 / 1e6 / secs[i],
            encoded as f64 / secs[i],
            rmse_sum[i] / rmse_n[i].max(1) as f64,
            rmse_worst[i],
            rmse_n[i],
        );
    }
    eprintln!(
        "speedup: {:.1}x (ispc_basic_prod vs dxtex_cpu_quick)",
        secs[0] / secs[1].max(1e-9)
    );
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
