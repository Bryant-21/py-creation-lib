use audio_native::pitch_shift::pitch_shift;
use ndarray::Array1;
use std::f32::consts::PI;

fn sine_wave(freq: f32, sr: f32, n: usize) -> Array1<f32> {
    (0..n)
        .map(|i| (2.0 * PI * freq * i as f32 / sr).sin())
        .collect()
}

#[test]
fn preserves_length_and_signal_across_semitone_range() {
    let input = sine_wave(440.0, 44100.0, 8192);
    let unchanged = pitch_shift(input.view(), 0.0);
    for i in 0..input.len() {
        assert!(
            (unchanged[i] - input[i]).abs() < 1e-5,
            "differ at {}: {} vs {}",
            i,
            unchanged[i],
            input[i],
        );
    }
    for semi in [-12.0_f32, -1.0, -0.5, 0.5, 1.0, 12.0] {
        let out = pitch_shift(input.view(), semi);
        assert_eq!(out.len(), input.len(), "length mismatch for {semi} semitones");
        let rms: f32 = (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
        assert!(rms > 0.01, "output too quiet for {semi} semitones: rms={rms}");
    }
}

#[test]
fn does_not_over_amplify_transients() {
    let mut input = Array1::<f32>::zeros(4096);
    input[0] = 1.0;
    for i in 1..200 {
        input[i] = 1.0 - (i as f32 / 199.0);
    }

    for semi in [-0.5_f32, 0.5_f32] {
        let out = pitch_shift(input.view(), semi);
        let peak = out.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(
            peak <= 2.0,
            "unexpected transient peak amplification for {} semitones: {}",
            semi,
            peak,
        );
    }
}
