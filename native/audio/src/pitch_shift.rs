//! Duration-preserving pitch shift via phase vocoder.
//!
//! Pipeline: STFT → frame-by-frame phase tracking → time-stretch by ratio
//! `r = 2^(semitones/12)` → linear-interpolation resample by `r` to bring
//! length back to the input length. Net effect: pitch shifted, duration
//! preserved.

use ndarray::{Array1, ArrayView1};
use rustfft::{FftPlanner, num_complex::Complex};
use std::f32::consts::PI;

const FRAME: usize = 2048;
const HOP_A: usize = 512;

fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect()
}

fn principal_arg(phase: f32) -> f32 {
    let two_pi = 2.0 * PI;
    let mut p = phase;
    while p > PI {
        p -= two_pi;
    }
    while p <= -PI {
        p += two_pi;
    }
    p
}

fn time_stretch(input: ArrayView1<f32>, stretch: f32) -> Vec<f32> {
    let win = hann_window(FRAME);
    let hop_a = HOP_A;
    let hop_s = ((HOP_A as f32) * stretch).round().max(1.0) as usize;
    let n_in = input.len();

    let n_frames = if n_in >= FRAME {
        (n_in - FRAME) / hop_a + 1
    } else {
        1
    };
    let n_out = (n_frames - 1) * hop_s + FRAME;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);
    let ifft = planner.plan_fft_inverse(FRAME);

    let mut out_buf = vec![0.0_f32; n_out];
    let mut out_weight = vec![0.0_f32; n_out];
    let mut last_phase = vec![0.0_f32; FRAME];
    let mut sum_phase = vec![0.0_f32; FRAME];

    let bin_freqs: Vec<f32> = (0..FRAME)
        .map(|k| 2.0 * PI * k as f32 / FRAME as f32)
        .collect();

    for f in 0..n_frames {
        let start = f * hop_a;
        let mut buf: Vec<Complex<f32>> = (0..FRAME)
            .map(|i| {
                let s = if start + i < n_in {
                    input[start + i]
                } else {
                    0.0
                };
                Complex::new(s * win[i], 0.0)
            })
            .collect();

        fft.process(&mut buf);

        for k in 0..FRAME {
            let mag = buf[k].norm();
            let phase = buf[k].arg();
            if f == 0 {
                last_phase[k] = phase;
                sum_phase[k] = phase;
                buf[k] = Complex::from_polar(mag, phase);
                continue;
            }
            let delta = phase - last_phase[k] - bin_freqs[k] * hop_a as f32;
            let delta = principal_arg(delta);
            let true_freq = bin_freqs[k] + delta / hop_a as f32;
            last_phase[k] = phase;
            sum_phase[k] += true_freq * hop_s as f32;
            buf[k] = Complex::from_polar(mag, sum_phase[k]);
        }

        ifft.process(&mut buf);

        let out_start = f * hop_s;
        for i in 0..FRAME {
            if out_start + i < n_out {
                // rustfft is unnormalized; the /FRAME here is the IFFT scale.
                out_buf[out_start + i] += buf[i].re * win[i] / FRAME as f32;
                out_weight[out_start + i] += win[i] * win[i];
            }
        }
    }

    for i in 0..n_out {
        if out_weight[i] > 1e-8 {
            out_buf[i] /= out_weight[i];
        }
    }

    out_buf
}

fn resample_linear(input: &[f32], ratio: f32) -> Vec<f32> {
    let n_in = input.len();
    if n_in == 0 {
        return Vec::new();
    }
    let n_out = ((n_in as f32) / ratio).round() as usize;
    let last = n_in - 1;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let pos = i as f32 * ratio;
        let idx = (pos.floor() as usize).min(last);
        let frac = pos - idx as f32;
        let a = input[idx];
        let b = input[(idx + 1).min(last)];
        out.push(a + (b - a) * frac);
    }
    out
}

/// Pitch-shift `samples` by `semitones`. Output length equals input length.
pub fn pitch_shift(samples: ArrayView1<f32>, semitones: f32) -> Array1<f32> {
    let target_len = samples.len();
    if semitones.abs() < 1e-6 || target_len == 0 {
        return samples.to_owned();
    }
    let ratio = 2.0_f32.powf(semitones / 12.0);
    let stretched = time_stretch(samples, ratio);
    let mut shifted = resample_linear(&stretched, ratio);
    shifted.resize(target_len, 0.0);
    Array1::from_vec(shifted)
}
