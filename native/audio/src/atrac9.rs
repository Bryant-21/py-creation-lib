use rustdct::{Dct4, DctPlanner};
use std::{fs, io::Write, path::Path, sync::Arc};

const FRAME_SAMPLES: usize = 256;
const ENCODER_DELAY: u32 = 256;
const SAMPLE_RATE: u32 = 48_000;
const SAMPLE_RATE_INDEX: u32 = 7;
const FRAMES_PER_SUPERFRAME: usize = 4;
const ATRAC9_SUBFORMAT_GUID: [u8; 16] = [
    0xd2, 0x42, 0xe1, 0x47, 0xba, 0x36, 0x8d, 0x4d, 0x88, 0xfc, 0x61, 0x65, 0x4f, 0x8c, 0x83, 0x6c,
];

pub fn encode_wav(source: &Path, output: &Path) -> Result<(), String> {
    let (samples, channels, source_rate) = read_wav(source)?;
    let samples = resample_interleaved(&samples, channels, source_rate, SAMPLE_RATE);
    let sample_count = samples.len() / channels;
    let frame_count = (sample_count + ENCODER_DELAY as usize)
        .div_ceil(FRAME_SAMPLES)
        .next_multiple_of(FRAMES_PER_SUPERFRAME);
    let frame_bytes = encoded_frame_bytes(channels);
    let mut data = Vec::with_capacity(frame_count * frame_bytes);
    let mut transforms = (0..channels).map(|_| Mdct::new()).collect::<Vec<_>>();

    for frame_index in 0..frame_count {
        let mut spectra = Vec::with_capacity(channels);
        for channel in 0..channels {
            let mut pcm = [0.0_f32; FRAME_SAMPLES];
            for (sample_index, value) in pcm.iter_mut().enumerate() {
                let source_index =
                    (frame_index * FRAME_SAMPLES + sample_index) * channels + channel;
                if let Some(sample) = samples.get(source_index) {
                    *value = *sample * 32_768.0;
                }
            }
            spectra.push(transforms[channel].run(&pcm));
        }
        data.extend_from_slice(&encode_frame(
            &spectra,
            channels,
            frame_bytes,
            frame_index % FRAMES_PER_SUPERFRAME == 0,
        )?);
        if frame_index % FRAMES_PER_SUPERFRAME == FRAMES_PER_SUPERFRAME - 1 {
            let superframe_bytes = frame_bytes * FRAMES_PER_SUPERFRAME;
            data.resize(data.len().next_multiple_of(superframe_bytes), 0);
        }
    }

    let wave = make_at9_wave(&data, channels, sample_count as u32, frame_bytes)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut file = fs::File::create(output).map_err(|error| error.to_string())?;
    file.write_all(&wave).map_err(|error| error.to_string())
}

fn read_wav(path: &Path) -> Result<(Vec<f32>, usize, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|error| error.to_string())?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    if !(1..=2).contains(&channels) {
        return Err(format!(
            "ATRAC9 encoding supports mono or stereo WAV files, got {channels} channels"
        ));
    }
    if spec.sample_rate == 0 {
        return Err("WAV sample rate must not be zero".to_owned());
    }
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|sample| sample.map(|value| value.clamp(-1.0, 1.0)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?,
        hound::SampleFormat::Int => {
            let scale = 2_f32.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| (value as f32 / scale).clamp(-1.0, 1.0)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        }
    };
    Ok((samples, channels, spec.sample_rate))
}

fn resample_interleaved(
    samples: &[f32],
    channels: usize,
    source_rate: u32,
    target_rate: u32,
) -> Vec<f32> {
    if source_rate == target_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let source_frames = samples.len() / channels;
    let target_frames = ((source_frames as u64 * target_rate as u64 + source_rate as u64 / 2)
        / source_rate as u64) as usize;
    let mut output = vec![0.0; target_frames * channels];
    for target_frame in 0..target_frames {
        let position = target_frame as f64 * source_rate as f64 / target_rate as f64;
        let left = (position.floor() as usize).min(source_frames - 1);
        let right = (left + 1).min(source_frames - 1);
        let fraction = (position - left as f64) as f32;
        for channel in 0..channels {
            let a = samples[left * channels + channel];
            let b = samples[right * channels + channel];
            output[target_frame * channels + channel] = a + (b - a) * fraction;
        }
    }
    output
}

struct Mdct {
    previous: [f32; FRAME_SAMPLES],
    window: [f32; FRAME_SAMPLES],
    dct: Arc<dyn Dct4<f32>>,
    scratch: Vec<f32>,
}

impl Mdct {
    fn new() -> Self {
        let mut planner = DctPlanner::new();
        let dct = planner.plan_dct4(FRAME_SAMPLES);
        let scratch = vec![0.0; dct.get_scratch_len()];
        let window = std::array::from_fn(|i| {
            let angle = ((i as f64 + 0.5) / FRAME_SAMPLES as f64 - 0.5) * std::f64::consts::PI;
            ((angle.sin() + 1.0) * 0.5) as f32
        });
        Self {
            previous: [0.0; FRAME_SAMPLES],
            window,
            dct,
            scratch,
        }
    }

    fn run(&mut self, input: &[f32; FRAME_SAMPLES]) -> [f32; FRAME_SAMPLES] {
        let half = FRAME_SAMPLES / 2;
        let mut output = [0.0_f32; FRAME_SAMPLES];
        for i in 0..half {
            let a = -self.window[half - i - 1] * input[half + i];
            let b = self.window[half + i] * input[half - i - 1];
            let c = self.window[i] * self.previous[i];
            let d = self.window[FRAME_SAMPLES - i - 1] * self.previous[FRAME_SAMPLES - i - 1];
            output[i] = a - b;
            output[half + i] = c - d;
        }
        self.dct
            .process_dct4_with_scratch(&mut output, &mut self.scratch);
        for value in &mut output {
            *value *= 2.0 / FRAME_SAMPLES as f32;
        }
        self.previous.copy_from_slice(input);
        output
    }
}

fn encoded_frame_bytes(channels: usize) -> usize {
    if channels == 1 { 96 } else { 192 }
}

fn encode_frame(
    spectra: &[[f32; FRAME_SAMPLES]],
    channels: usize,
    frame_bytes: usize,
    first_in_superframe: bool,
) -> Result<Vec<u8>, String> {
    let band_count = 8;
    let extension_units = 16;
    let gradient = if channels == 1 { 22 } else { 21 };
    let base_scale_factor = if channels == 1 { 31 } else { 30 };
    let mut writer = BitWriter::new(frame_bytes);
    writer.write(u32::from(!first_in_superframe), 1)?;
    writer.write(0, 1)?;
    writer.write(band_count - 3, 4)?;
    if channels == 2 {
        writer.write(8 - 3, 4)?;
    }
    writer.write(0, 1)?;
    writer.write(0, 2)?;
    writer.write(1, 6)?;
    writer.write(0, 6)?;
    writer.write(gradient, 5)?;
    writer.write(gradient, 5)?;
    writer.write(0, 4)?;
    if channels == 2 {
        writer.write(0, 1)?;
        writer.write(0, 1)?;
    }
    writer.write(1, 1)?;
    writer.write(0, 2)?;
    let extension_bits = if channels == 1 { 1 } else { 23 };
    writer.write(extension_bits, 5)?;
    for _ in 0..extension_bits {
        writer.write(0, 1)?;
    }

    for (channel, spectrum) in spectra.iter().enumerate() {
        let scale_factor = base_scale_factor + u32::from(channel > 0);
        let precision_bits = scale_factor - gradient + 1;
        let coded_coefficients = 64;
        writer.write(1, 2)?;
        if channel == 0 {
            writer.write(3, 2)?;
            for _ in 0..extension_units {
                writer.write(scale_factor, 5)?;
            }
        } else {
            writer.write(0, 2)?;
            for _ in 0..extension_units {
                writer.write(0b11, 2)?;
            }
        }
        let step = 2.0 / ((1_u32 << precision_bits) - 1) as f32;
        let spectrum_scale = 2_f32.powi(scale_factor as i32 - 15);
        for value in spectrum.iter().take(coded_coefficients) {
            let limit = (1_u32 << (precision_bits - 1)) as f32;
            let quantized = (*value / (step * spectrum_scale))
                .round()
                .clamp(-limit, limit - 1.0) as i32;
            writer.write(quantized as u32, precision_bits)?;
        }
    }
    Ok(writer.finish())
}

fn make_at9_wave(
    data: &[u8],
    channels: usize,
    sample_count: u32,
    frame_bytes: usize,
) -> Result<Vec<u8>, String> {
    let channel_config = if channels == 1 { 0 } else { 2 };
    let config_value = (0xfe_u32 << 24)
        | (SAMPLE_RATE_INDEX << 20)
        | (channel_config << 17)
        | (((frame_bytes as u32 - 1) & 0x7ff) << 5)
        | (2 << 3);
    let config = config_value.to_be_bytes();
    let avg_bytes_per_second = frame_bytes as u32 * SAMPLE_RATE / FRAME_SAMPLES as u32;
    let channel_mask = if channels == 1 { 4_u32 } else { 3_u32 };

    let mut fmt = Vec::with_capacity(52);
    push_u16(&mut fmt, 0xfffe);
    push_u16(&mut fmt, channels as u16);
    push_u32(&mut fmt, SAMPLE_RATE);
    push_u32(&mut fmt, avg_bytes_per_second);
    push_u16(&mut fmt, (frame_bytes * FRAMES_PER_SUPERFRAME) as u16);
    push_u16(&mut fmt, 0);
    push_u16(&mut fmt, 34);
    push_u16(&mut fmt, 1024);
    push_u32(&mut fmt, channel_mask);
    fmt.extend_from_slice(&ATRAC9_SUBFORMAT_GUID);
    push_u32(&mut fmt, 1);
    fmt.extend_from_slice(&config);
    push_u32(&mut fmt, 0);

    let mut fact = Vec::with_capacity(12);
    push_u32(&mut fact, sample_count);
    push_u32(&mut fact, ENCODER_DELAY);
    push_u32(&mut fact, ENCODER_DELAY);

    let riff_size = 4 + 8 + fmt.len() + 8 + fact.len() + 8 + data.len();
    let riff_size =
        u32::try_from(riff_size).map_err(|_| "ATRAC9 output exceeds RIFF size limit")?;
    let data_size =
        u32::try_from(data.len()).map_err(|_| "ATRAC9 audio data exceeds RIFF size limit")?;
    let mut wave = Vec::with_capacity(riff_size as usize + 8);
    wave.extend_from_slice(b"RIFF");
    push_u32(&mut wave, riff_size);
    wave.extend_from_slice(b"WAVEfmt ");
    push_u32(&mut wave, fmt.len() as u32);
    wave.extend_from_slice(&fmt);
    wave.extend_from_slice(b"fact");
    push_u32(&mut wave, fact.len() as u32);
    wave.extend_from_slice(&fact);
    wave.extend_from_slice(b"data");
    push_u32(&mut wave, data_size);
    wave.extend_from_slice(data);
    Ok(wave)
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

struct BitWriter {
    bytes: Vec<u8>,
    position: usize,
}

impl BitWriter {
    fn new(size: usize) -> Self {
        Self {
            bytes: vec![0; size],
            position: 0,
        }
    }

    fn write(&mut self, value: u32, bit_count: u32) -> Result<(), String> {
        if self.position + bit_count as usize > self.bytes.len() * 8 {
            return Err("ATRAC9 frame exceeded configured frame size".to_owned());
        }
        for shift in (0..bit_count).rev() {
            let bit = (value >> shift) & 1;
            self.bytes[self.position / 8] |= (bit as u8) << (7 - self.position % 8);
            self.position += 1;
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<u8> {
        self.bytes.truncate(self.position.div_ceil(8));
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_sizes_fit_config_field() {
        assert_eq!(encoded_frame_bytes(1), 96);
        assert_eq!(encoded_frame_bytes(2), 192);
        assert!(encoded_frame_bytes(2) <= 2048);
        let silent = [0.0; FRAME_SAMPLES];
        assert_eq!(encode_frame(&[silent], 1, 96, true).unwrap().len(), 96);
        assert_eq!(
            encode_frame(&[silent, silent], 2, 192, true).unwrap().len(),
            192
        );
    }

    #[test]
    fn config_contains_frame_size_and_channel_mapping() {
        for channels in [1, 2] {
            let frame_bytes = encoded_frame_bytes(channels);
            let wave = make_at9_wave(&vec![0; frame_bytes], channels, 1, frame_bytes).unwrap();
            let config = u32::from_be_bytes(wave[64..68].try_into().unwrap());
            assert_eq!((config >> 24) & 0xff, 0xfe);
            assert_eq!(((config >> 5) & 0x7ff) + 1, frame_bytes as u32);
            assert_eq!((config >> 3) & 3, 2);
        }
    }
}
