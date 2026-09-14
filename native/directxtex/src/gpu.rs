//! GPU BC7 compression via DirectXTex DirectCompute, with a process-global
//! Mutex-guarded device. The GPU is one physical resource: serializing
//! submissions is correct, and one GPU BC7 encode beats N CPU cores.

use std::os::raw::c_void;
use std::sync::{Mutex, OnceLock};

unsafe extern "C" {
    fn DirectXTexFFI_GpuCreateDevice() -> *mut c_void;
    #[allow(dead_code)]
    fn DirectXTexFFI_GpuReleaseDevice(dev: *mut c_void);
    fn DirectXTexFFI_GpuCompressBC7(
        dev: *mut c_void,
        rgba: *const u8,
        w: u32,
        h: u32,
        srgb: i32,
        out: *mut u8,
        out_len: usize,
    ) -> i32;
    fn DirectXTexFFI_GpuCompressBC7Batch(
        dev: *mut c_void,
        n: u32,
        widths: *const u32,
        heights: *const u32,
        rgba: *const *const u8,
        srgb: i32,
        out: *const *mut u8,
        out_len: *const usize,
    ) -> i32;
}

/// Opaque device pointer. Safe to send because every access is serialized
/// behind the `Mutex` below; the device is never touched concurrently.
struct Device(*mut c_void);
unsafe impl Send for Device {}

enum GpuState {
    Available(Device),
    Unavailable,
}

fn gpu_state() -> &'static Mutex<GpuState> {
    static STATE: OnceLock<Mutex<GpuState>> = OnceLock::new();
    STATE.get_or_init(|| {
        let dev = unsafe { DirectXTexFFI_GpuCreateDevice() };
        if dev.is_null() {
            Mutex::new(GpuState::Unavailable)
        } else {
            Mutex::new(GpuState::Available(Device(dev)))
        }
    })
}

fn bc7_payload_len(width: u32, height: u32) -> usize {
    let bx = width.div_ceil(4) as usize;
    let by = height.div_ceil(4) as usize;
    bx * by * 16
}

/// Compress one RGBA8 image to a BC7 DDS-block payload (no header).
/// Returns `Err` if the GPU is unavailable or the encode fails — callers
/// must fall back to the CPU encoder.
pub fn compress_bc7_gpu(
    rgba: &[u8],
    width: u32,
    height: u32,
    srgb: bool,
) -> Result<Vec<u8>, String> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected {
        return Err(format!("rgba len {} != expected {expected}", rgba.len()));
    }
    let _wait = crate::profiling::Timer::new(crate::profiling::Stage::GpuWait);
    let guard = gpu_state()
        .lock()
        .map_err(|_| "gpu mutex poisoned".to_string())?;
    let Device(dev) = match &*guard {
        GpuState::Available(d) => d,
        GpuState::Unavailable => return Err("gpu device unavailable".to_string()),
    };
    let out_len = bc7_payload_len(width, height);
    let mut out = vec![0u8; out_len];
    let rc = unsafe {
        DirectXTexFFI_GpuCompressBC7(
            *dev,
            rgba.as_ptr(),
            width,
            height,
            i32::from(srgb),
            out.as_mut_ptr(),
            out_len,
        )
    };
    if rc != 0 {
        return Err(format!("gpu bc7 compress failed: rc={rc}"));
    }
    Ok(out)
}

/// Compress a batch of RGBA8 images — typically one texture's whole mip chain —
/// to BC7 in a single GPU submission that reuses a cached compute-shader pipeline.
/// `images` is `(width, height, rgba)` per level; the returned `Vec` holds one
/// BC7 payload per level in the same order. Returns `Err` if the GPU is
/// unavailable or any encode fails — callers must fall back to the CPU encoder.
pub fn compress_bc7_gpu_batch(
    images: &[(u32, u32, &[u8])],
    srgb: bool,
) -> Result<Vec<Vec<u8>>, String> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let n = u32::try_from(images.len()).map_err(|_| "image count exceeds u32".to_string())?;

    let mut widths = Vec::with_capacity(images.len());
    let mut heights = Vec::with_capacity(images.len());
    let mut rgba_ptrs = Vec::with_capacity(images.len());
    let mut outs: Vec<Vec<u8>> = Vec::with_capacity(images.len());
    for (w, h, rgba) in images {
        let expected = (*w as usize) * (*h as usize) * 4;
        if rgba.len() != expected {
            return Err(format!("rgba len {} != expected {expected}", rgba.len()));
        }
        widths.push(*w);
        heights.push(*h);
        rgba_ptrs.push(rgba.as_ptr());
        outs.push(vec![0u8; bc7_payload_len(*w, *h)]);
    }
    let out_ptrs: Vec<*mut u8> = outs.iter_mut().map(|v| v.as_mut_ptr()).collect();
    let out_lens: Vec<usize> = outs.iter().map(Vec::len).collect();

    let _wait = crate::profiling::Timer::new(crate::profiling::Stage::GpuWait);
    let guard = gpu_state()
        .lock()
        .map_err(|_| "gpu mutex poisoned".to_string())?;
    let Device(dev) = match &*guard {
        GpuState::Available(d) => d,
        GpuState::Unavailable => return Err("gpu device unavailable".to_string()),
    };
    let rc = unsafe {
        DirectXTexFFI_GpuCompressBC7Batch(
            *dev,
            n,
            widths.as_ptr(),
            heights.as_ptr(),
            rgba_ptrs.as_ptr(),
            i32::from(srgb),
            out_ptrs.as_ptr(),
            out_lens.as_ptr(),
        )
    };
    if rc != 0 {
        return Err(format!("gpu bc7 batch compress failed: rc={rc}"));
    }
    Ok(outs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_bc7_roundtrips_or_reports_unavailable() {
        // 8x8 solid red, RGBA8.
        let rgba = vec![255u8, 0, 0, 255].repeat(8 * 8);
        match compress_bc7_gpu(&rgba, 8, 8, false) {
            Ok(bytes) => {
                // BC7 = 16 bytes per 4x4 block; 8x8 = 4 blocks = 64 bytes.
                assert_eq!(bytes.len(), 64, "BC7 payload size for 8x8");
            }
            Err(e) => {
                // Acceptable in a GPU-less CI box: must be a clean error, not a panic.
                assert!(!e.is_empty(), "error message present");
            }
        }
    }

    #[test]
    fn gpu_bc7_batch_matches_per_image() {
        // A descending mip chain with varied content per level.
        let sizes = [(8u32, 8u32), (4, 4), (2, 2), (1, 1)];
        let images: Vec<(u32, u32, Vec<u8>)> = sizes
            .iter()
            .enumerate()
            .map(|(k, (w, h))| {
                let px = (0..(*w as usize) * (*h as usize))
                    .flat_map(|i| [(i * 7 + k) as u8, (i * 13) as u8, (i * 29) as u8, 255])
                    .collect();
                (*w, *h, px)
            })
            .collect();

        for &srgb in &[false, true] {
            // Reference: the per-image path, one submission per level.
            let mut per_image = Vec::new();
            let mut gpu_ok = true;
            for (w, h, px) in &images {
                match compress_bc7_gpu(px, *w, *h, srgb) {
                    Ok(bytes) => per_image.push(bytes),
                    Err(_) => {
                        gpu_ok = false;
                        break;
                    }
                }
            }
            if !gpu_ok {
                continue; // No usable GPU on this box; nothing to compare against.
            }

            let refs: Vec<(u32, u32, &[u8])> = images
                .iter()
                .map(|(w, h, px)| (*w, *h, px.as_slice()))
                .collect();
            let batched = compress_bc7_gpu_batch(&refs, srgb)
                .expect("batch must succeed when per-image succeeded");

            assert_eq!(batched.len(), per_image.len());
            for (got, want) in batched.iter().zip(per_image.iter()) {
                assert_eq!(
                    got, want,
                    "batch payload must byte-match per-image (srgb={srgb})"
                );
            }
        }
    }
}
