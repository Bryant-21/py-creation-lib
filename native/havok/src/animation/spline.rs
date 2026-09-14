use crate::error::{HavokError, HavokResult};

// ---------------------------------------------------------------------------
// Public constants (mirror Python _ROTATION_SIZE / _ROTATION_ALIGN)
// Index order matches RotationQuantization discriminant: Polar32=0 … Uncompressed=5
// ---------------------------------------------------------------------------

pub const ROTATION_SIZE: [usize; 6] = [4, 5, 6, 3, 2, 16];
pub const ROTATION_ALIGN: [usize; 6] = [4, 1, 2, 1, 2, 4];

// Bytes per scalar quantization type: BITS8=0 → 1, BITS16=1 → 2
const SCALAR_SIZE: [usize; 2] = [1, 2];

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationQuantization {
    Polar32 = 0,
    ThreeComp40 = 1,
    ThreeComp48 = 2,
    ThreeComp24 = 3,
    Straight16 = 4,
    Uncompressed = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarQuantization {
    Bits8 = 0,
    Bits16 = 1,
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SplineTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct SplineFrame {
    pub transforms: Vec<SplineTransform>,
}

// ---------------------------------------------------------------------------
// Public quaternion functions
// ---------------------------------------------------------------------------

pub fn unpack_uncompressed_quat(data: &[u8], offset: usize) -> HavokResult<[f32; 4]> {
    ensure_len(data, offset + 16, "uncompressed quaternion")?;
    Ok([
        read_f32_le(data, offset),
        read_f32_le(data, offset + 4),
        read_f32_le(data, offset + 8),
        read_f32_le(data, offset + 12),
    ])
}

pub fn unpack_straight16_quat(data: &[u8], offset: usize) -> HavokResult<[f32; 4]> {
    ensure_len(data, offset + 2, "straight16 quaternion")?;
    let b0 = data[offset];
    let b1 = data[offset + 1];
    normalize([
        f32::from(b0 & 0x0F) / 7.0 - 1.0,
        f32::from(b0 >> 4) / 7.0 - 1.0,
        f32::from(b1 & 0x0F) / 7.0 - 1.0,
        f32::from(b1 >> 4) / 7.0 - 1.0,
    ])
}

pub fn pack_straight16_quat(quat: [f32; 4]) -> HavokResult<[u8; 2]> {
    let encoded = quat.map(|value| ((value.clamp(-1.0, 1.0) + 1.0) * 7.0).round() as u8);
    Ok([
        (encoded[0] & 0x0F) | ((encoded[1] & 0x0F) << 4),
        (encoded[2] & 0x0F) | ((encoded[3] & 0x0F) << 4),
    ])
}

// ---------------------------------------------------------------------------
// Quaternion unpackers
// ---------------------------------------------------------------------------

/// POLAR32: 4 bytes → quaternion [x, y, z, w]
pub fn unpack_polar32(data: &[u8], off: usize) -> HavokResult<[f32; 4]> {
    ensure_len(data, off + 4, "polar32 quaternion")?;
    let val = read_u32_le(data, off);
    let e = val & 0x0003_FFFF;
    let iw = (val >> 18) & 0x03FF;
    let signs = val >> 28;

    let w = 1.0 - (iw as f32 / 1023.0).powi(2);

    let ipitch = (e as f32).sqrt() as u32;
    let iyaw = e - ipitch * ipitch;
    let pitch = (ipitch as f32 / 511.0) * (std::f32::consts::PI / 2.0);
    let nyaw = 2 * ipitch;
    let yaw = if nyaw != 0 {
        (iyaw as f32 / nyaw as f32) * (std::f32::consts::PI / 2.0)
    } else {
        0.0
    };

    let x = yaw.cos() * pitch.sin();
    let y = yaw.sin() * pitch.sin();
    let z = pitch.cos();

    let mag = (1.0f32 - w * w).max(0.0).sqrt();
    let mut q = [x * mag, y * mag, z * mag, w];
    if signs & 1 != 0 {
        q[0] = -q[0];
    }
    if signs & 2 != 0 {
        q[1] = -q[1];
    }
    if signs & 4 != 0 {
        q[2] = -q[2];
    }
    if signs & 8 != 0 {
        q[3] = -q[3];
    }
    Ok(q)
}

/// THREECOMP40: 5 bytes → quaternion (smallest-three, 12-bit)
pub fn unpack_threecomp40(data: &[u8], off: usize) -> HavokResult<[f32; 4]> {
    ensure_len(data, off + 5, "threecomp40 quaternion")?;
    let b = &data[off..off + 5];
    let a = (b[0] as u32) | (((b[1] & 0x0F) as u32) << 8);
    let bv = ((b[1] >> 4) as u32) | ((b[2] as u32) << 4);
    let c = (b[3] as u32) | (((b[4] & 0x0F) as u32) << 8);
    let maxi = ((b[4] >> 4) & 0x03) as usize;
    let sign_neg = (b[4] & 0x40) != 0;

    let scale = 1.0 / (2047.0 * 2.0f32.sqrt());
    let vals = [
        (a as f32 - 2047.0) * scale,
        (bv as f32 - 2047.0) * scale,
        (c as f32 - 2047.0) * scale,
    ];

    Ok(smallest_three_reconstruct(&vals, maxi, sign_neg))
}

/// THREECOMP48: 6 bytes → quaternion (smallest-three, 15-bit)
pub fn unpack_threecomp48(data: &[u8], off: usize) -> HavokResult<[f32; 4]> {
    ensure_len(data, off + 6, "threecomp48 quaternion")?;
    let w0 = read_u16_le(data, off) as u32;
    let w1 = read_u16_le(data, off + 2) as u32;
    let w2 = read_u16_le(data, off + 4) as u32;

    let a = w0 & 0x7FFF;
    let bv = w1 & 0x7FFF;
    let c = w2 & 0x7FFF;
    let maxi = (((w0 >> 15) & 1) | ((w1 >> 14) & 2)) as usize;
    let sign_neg = (w2 & 0x8000) != 0;

    let scale = 1.0 / (16383.0 * 2.0f32.sqrt());
    let vals = [
        (a as f32 - 16383.0) * scale,
        (bv as f32 - 16383.0) * scale,
        (c as f32 - 16383.0) * scale,
    ];

    Ok(smallest_three_reconstruct(&vals, maxi, sign_neg))
}

/// THREECOMP24: 3 bytes → quaternion (smallest-three, 7-bit)
pub fn unpack_threecomp24(data: &[u8], off: usize) -> HavokResult<[f32; 4]> {
    ensure_len(data, off + 3, "threecomp24 quaternion")?;
    let b = &data[off..off + 3];
    let a = (b[0] & 0x7F) as u32;
    let bv = (b[1] & 0x7F) as u32;
    let c = (b[2] & 0x7F) as u32;
    let maxi = ((b[0] >> 7) as usize) | (((b[1] >> 6) & 0x02) as usize);
    let sign_neg = (b[2] & 0x80) != 0;

    let scale = 1.0 / (63.0 * 2.0f32.sqrt());
    let vals = [
        (a as f32 - 63.0) * scale,
        (bv as f32 - 63.0) * scale,
        (c as f32 - 63.0) * scale,
    ];

    Ok(smallest_three_reconstruct(&vals, maxi, sign_neg))
}

// ---------------------------------------------------------------------------
// Scalar dequantization
// ---------------------------------------------------------------------------

pub fn dequant_u8(min: f32, max: f32, value: u8) -> f32 {
    value as f32 * (1.0 / 255.0) * (max - min) + min
}

pub fn dequant_u16(min: f32, max: f32, value: u16) -> f32 {
    value as f32 * (1.0 / 65535.0) * (max - min) + min
}

// ---------------------------------------------------------------------------
// Knot span search  (public for testing)
// ---------------------------------------------------------------------------

/// Find knot span index i such that knots[base+i] <= u < knots[base+i+1].
pub fn find_span(n: u32, p: u32, u: u32, knots: &[u8], base: usize) -> usize {
    let n = n as usize;
    let p = p as usize;
    let u = u as usize;

    if u >= knots[base + n + 1] as usize {
        return n;
    }
    if u <= knots[base] as usize {
        return p;
    }

    let mut low = p;
    let mut high = n + 1;
    let mut mid = (low + high) / 2;
    while (u < knots[base + mid] as usize) || (u >= knots[base + mid + 1] as usize) {
        if u < knots[base + mid] as usize {
            high = mid;
        } else {
            low = mid;
        }
        mid = (low + high) / 2;
    }
    mid
}

// ---------------------------------------------------------------------------
// B-spline evaluators (degree 1–3 matching Python; degree 4 unused but included)
// ---------------------------------------------------------------------------

fn evaluate_degree1(u: f32, cap_u: &[f32], p: &[[f32; 4]]) -> [f32; 4] {
    let left = u - cap_u[0];
    let right = cap_u[1] - u;
    let denom = right + left;
    if denom.abs() < 1e-30 {
        return p[0];
    }
    let t = left / denom;
    std::array::from_fn(|i| p[0][i] + t * (p[1][i] - p[0][i]))
}

fn evaluate_degree2(u: f32, cap_u: &[f32], p: &[[f32; 4]]) -> [f32; 4] {
    let left1 = u - cap_u[1];
    let right1 = cap_u[2] - u;
    let d1 = right1 + left1;
    let (n0, n1) = if d1.abs() < 1e-30 {
        (0.5, 0.5)
    } else {
        let inv = 1.0 / d1;
        (right1 * inv, left1 * inv)
    };

    let left2 = u - cap_u[0];
    let right2 = cap_u[3] - u;

    let d2 = right1 + left2;
    let (temp0_r, temp0_l) = if d2.abs() < 1e-30 {
        (0.5 * n0, 0.5 * n0)
    } else {
        let inv2 = n0 / d2;
        (right1 * inv2, left2 * inv2)
    };

    let d3 = right2 + left1;
    let (temp1_r, temp1_l) = if d3.abs() < 1e-30 {
        (0.5 * n1, 0.5 * n1)
    } else {
        let inv3 = n1 / d3;
        (right2 * inv3, left1 * inv3)
    };

    let b0 = temp0_r;
    let b1 = temp0_l + temp1_r;
    let b2 = temp1_l;

    std::array::from_fn(|i| b0 * p[0][i] + b1 * p[1][i] + b2 * p[2][i])
}

fn evaluate_degree3(u: f32, cap_u: &[f32], p: &[[f32; 4]]) -> [f32; 4] {
    // j=1
    let left1 = u - cap_u[2];
    let right1 = cap_u[3] - u;
    let d1 = right1 + left1;
    let (n0, n1) = if d1.abs() < 1e-30 {
        (0.5, 0.5)
    } else {
        let inv = 1.0 / d1;
        (right1 * inv, left1 * inv)
    };

    // j=2
    let left2 = u - cap_u[1];
    let right2 = cap_u[4] - u;

    let d = right1 + left2;
    let t0 = if d.abs() > 1e-30 { n0 / d } else { 0.0 };
    let n00 = right1 * t0;
    let saved = left2 * t0;

    let d = right2 + left1;
    let t1 = if d.abs() > 1e-30 { n1 / d } else { 0.0 };
    let n01 = saved + right2 * t1;
    let n02 = left1 * t1;

    // j=3
    let left3 = u - cap_u[0];
    let right3 = cap_u[5] - u;

    let d = right1 + left3;
    let t0 = if d.abs() > 1e-30 { n00 / d } else { 0.0 };
    let b0 = right1 * t0;
    let saved = left3 * t0;

    let d = right2 + left2;
    let t1 = if d.abs() > 1e-30 { n01 / d } else { 0.0 };
    let b1 = saved + right2 * t1;
    let saved2 = left2 * t1;

    let d = right3 + left1;
    let t2 = if d.abs() > 1e-30 { n02 / d } else { 0.0 };
    let b2 = saved2 + right3 * t2;
    let b3 = left1 * t2;

    std::array::from_fn(|i| b0 * p[0][i] + b1 * p[1][i] + b2 * p[2][i] + b3 * p[3][i])
}

// ---------------------------------------------------------------------------
// Reader (cursor over a byte buffer)
// ---------------------------------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], pos: usize) -> Self {
        Self { buf, pos }
    }

    fn u8(&mut self) -> HavokResult<u8> {
        if self.pos >= self.buf.len() {
            return Err(HavokError::InvalidInput("reader underflow (u8)".into()));
        }
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn u16(&mut self) -> HavokResult<u16> {
        if self.pos + 2 > self.buf.len() {
            return Err(HavokError::InvalidInput("reader underflow (u16)".into()));
        }
        let v = read_u16_le(self.buf, self.pos);
        self.pos += 2;
        Ok(v)
    }

    fn f32(&mut self) -> HavokResult<f32> {
        if self.pos + 4 > self.buf.len() {
            return Err(HavokError::InvalidInput("reader underflow (f32)".into()));
        }
        let v = read_f32_le(self.buf, self.pos);
        self.pos += 4;
        Ok(v)
    }

    fn align(&mut self, n: usize) {
        self.pos = (self.pos + n - 1) & !(n - 1);
    }
}

// ---------------------------------------------------------------------------
// Knot reading helper
// ---------------------------------------------------------------------------

fn read_knots(
    r: &mut Reader<'_>,
    quantized_time: u32,
    frame_duration: f32,
) -> HavokResult<(usize, usize, usize, Vec<f32>)> {
    let n = r.u16()? as usize;
    let p = r.u8()? as usize;
    let m = n + p + 1;
    let knot_base = r.pos;
    let knot_end = knot_base
        .checked_add(m + 1)
        .ok_or_else(|| HavokError::InvalidInput("spline knot range overflow".into()))?;
    if p > n || knot_end > r.buf.len() {
        return Err(HavokError::InvalidInput(format!(
            "spline knot payload is truncated or invalid (n={n}, degree={p}, bytes={}..{knot_end}, data_len={})",
            knot_base,
            r.buf.len()
        )));
    }

    let span = find_span(n as u32, p as u32, quantized_time, r.buf, knot_base);

    let mut cap_u = vec![0.0f32; 2 * p];
    for j in 0..2 * p {
        let idx = knot_base + (span - p + 1) + j;
        cap_u[j] = r.buf[idx] as f32 * frame_duration;
    }

    r.pos = knot_end;
    Ok((n, p, span, cap_u))
}

// ---------------------------------------------------------------------------
// Scalar track sampling
// ---------------------------------------------------------------------------

fn sample_scalar_track(
    r: &mut Reader<'_>,
    scalar_q: usize,
    quantized_time: u32,
    frame_duration: f32,
    u: f32,
    mask: u8,
    identity: [f32; 3],
) -> HavokResult<[f32; 3]> {
    let has_dynamic = (mask & 0xF0) != 0;

    let (n, p, span, cap_u) = if has_dynamic {
        let (n, p, span, ku) = read_knots(r, quantized_time, frame_duration)?;
        (n, p, span, ku)
    } else {
        (0, 0, 0, vec![])
    };

    r.align(4);

    let mut stat = [0.0f32; 3];
    let mut minp = [0.0f32; 3];
    let mut maxp = [0.0f32; 3];

    for j in 0..3 {
        if mask & (1 << j) != 0 {
            // static component
            stat[j] = r.f32()?;
        } else if mask & (1 << (j + 4)) != 0 {
            // dynamic component — read bounds
            minp[j] = r.f32()?;
            maxp[j] = r.f32()?;
        }
    }

    if !has_dynamic {
        let mut out = identity;
        for j in 0..3 {
            if mask & (1 << j) != 0 {
                out[j] = stat[j];
            }
        }
        r.align(4);
        return Ok(out);
    }

    let num_dyn = popcount_3(mask >> 4);
    let bpc = *SCALAR_SIZE.get(scalar_q).ok_or_else(|| {
        HavokError::InvalidInput(format!(
            "unknown spline scalar quantization type {scalar_q}"
        ))
    })?;

    r.align(2);
    let cp_start = r.pos;
    let cp_bytes = (n + 1)
        .checked_mul(num_dyn)
        .and_then(|count| count.checked_mul(bpc))
        .ok_or_else(|| HavokError::InvalidInput("spline scalar payload size overflow".into()))?;
    let cp_end = cp_start
        .checked_add(cp_bytes)
        .ok_or_else(|| HavokError::InvalidInput("spline scalar payload range overflow".into()))?;
    if cp_end > r.buf.len() {
        return Err(HavokError::InvalidInput(format!(
            "spline scalar control-point payload is truncated (bytes={cp_start}..{cp_end}, data_len={})",
            r.buf.len()
        )));
    }

    let mut points: Vec<[f32; 4]> = Vec::with_capacity(p + 1);
    for i in 0..=p {
        let cp_idx = span - p + i;
        let mut point = [identity[0], identity[1], identity[2], 0.0];
        let off = cp_start + cp_idx * num_dyn * bpc;
        let mut dyn_j = 0usize;
        for j in 0..3 {
            if mask & (1 << (j + 4)) != 0 {
                point[j] = if scalar_q == 0 {
                    // BITS8
                    dequant_u8(minp[j], maxp[j], r.buf[off + dyn_j])
                } else {
                    // BITS16
                    let v = read_u16_le(r.buf, off + dyn_j * 2);
                    dequant_u16(minp[j], maxp[j], v)
                };
                dyn_j += 1;
            } else if mask & (1 << j) != 0 {
                point[j] = stat[j];
            }
        }
        points.push(point);
    }

    r.pos = cp_end;
    r.align(4);

    let result4 = evaluate_bspline(p, u, &cap_u, &points);
    Ok([result4[0], result4[1], result4[2]])
}

// ---------------------------------------------------------------------------
// Rotation track sampling
// ---------------------------------------------------------------------------

fn sample_rotation_track(
    r: &mut Reader<'_>,
    rot_q: usize,
    quantized_time: u32,
    frame_duration: f32,
    u: f32,
    mask: u8,
) -> HavokResult<[f32; 4]> {
    if rot_q >= ROTATION_SIZE.len() {
        return Err(HavokError::InvalidInput(format!(
            "unknown rotation quantization type {rot_q}"
        )));
    }
    let has_dynamic = (mask & 0xF0) != 0;
    let has_static = (mask & 0x0F) != 0;

    let result = if has_dynamic {
        let (n, p, span, cap_u) = read_knots(r, quantized_time, frame_duration)?;
        r.align(ROTATION_ALIGN[rot_q]);
        let bpq = ROTATION_SIZE[rot_q];
        let cp_start = r.pos;

        let mut points: Vec<[f32; 4]> = Vec::with_capacity(p + 1);
        for i in 0..=p {
            let cp_idx = span - p + i;
            let off = cp_start + cp_idx * bpq;
            let q = unpack_quat_by_type(rot_q, r.buf, off)?;
            points.push(q);
        }

        let cp_end = cp_start
            .checked_add((n + 1).checked_mul(bpq).ok_or_else(|| {
                HavokError::InvalidInput("spline rotation payload size overflow".into())
            })?)
            .ok_or_else(|| {
                HavokError::InvalidInput("spline rotation payload range overflow".into())
            })?;
        if cp_end > r.buf.len() {
            return Err(HavokError::InvalidInput(format!(
                "spline rotation control-point payload is truncated (bytes={cp_start}..{cp_end}, data_len={})",
                r.buf.len()
            )));
        }
        r.pos = cp_end;

        let raw = evaluate_bspline(p, u, &cap_u, &points);
        let norm: f32 = raw.iter().map(|c| c * c).sum::<f32>().sqrt();
        if norm > 1e-10 {
            [raw[0] / norm, raw[1] / norm, raw[2] / norm, raw[3] / norm]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    } else if has_static {
        r.align(ROTATION_ALIGN[rot_q]);
        let q = unpack_quat_by_type(rot_q, r.buf, r.pos)?;
        r.pos += ROTATION_SIZE[rot_q];
        q
    } else {
        [0.0, 0.0, 0.0, 1.0]
    };

    r.align(4);
    Ok(result)
}

// ---------------------------------------------------------------------------
// B-spline dispatch
// ---------------------------------------------------------------------------

fn evaluate_bspline(p: usize, u: f32, cap_u: &[f32], points: &[[f32; 4]]) -> [f32; 4] {
    match p {
        1 => evaluate_degree1(u, cap_u, points),
        2 => evaluate_degree2(u, cap_u, points),
        3 => evaluate_degree3(u, cap_u, points),
        _ => {
            // degree 0 or unsupported: return first point
            if points.is_empty() {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                points[0]
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Quaternion unpacker dispatch
// ---------------------------------------------------------------------------

fn unpack_quat_by_type(rot_q: usize, data: &[u8], off: usize) -> HavokResult<[f32; 4]> {
    match rot_q {
        0 => unpack_polar32(data, off),
        1 => unpack_threecomp40(data, off),
        2 => unpack_threecomp48(data, off),
        3 => unpack_threecomp24(data, off),
        4 => unpack_straight16_quat(data, off),
        5 => unpack_uncompressed_quat(data, off),
        _ => Err(HavokError::InvalidInput(format!(
            "unknown rotation quantization type {rot_q}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Top-level decompressor
// ---------------------------------------------------------------------------

/// Decompress a spline-compressed animation into per-frame transforms.
///
/// Returns `Vec<SplineFrame>` with one entry per frame; each frame contains
/// `num_tracks` `SplineTransform` values.
#[allow(clippy::too_many_arguments)]
pub fn decompress_spline(
    data: &[u8],
    num_tracks: u32,
    num_floats: u32,
    num_frames: u32,
    max_frames_per_block: u32,
    num_blocks: u32,
    block_offsets: &[u32],
    float_block_offsets: &[u32],
    mask_and_quant_size: u32,
    block_duration: f32,
    block_inverse_duration: f32,
    frame_duration: f32,
) -> HavokResult<Vec<SplineFrame>> {
    decompress_spline_full(
        data,
        num_tracks,
        num_floats,
        num_frames,
        max_frames_per_block,
        num_blocks,
        block_offsets,
        float_block_offsets,
        mask_and_quant_size,
        block_duration,
        block_inverse_duration,
        frame_duration,
    )
    .map(|d| d.frames)
}

/// Frames + float tracks recovered by `decompress_spline_full`.
#[derive(Debug, Clone)]
pub struct DecompressedSpline {
    pub frames: Vec<SplineFrame>,
    /// `float_tracks[track_idx][frame_idx]` — per-frame value for each float track.
    /// Empty when the source animation has no float tracks.
    pub float_tracks: Vec<Vec<f32>>,
}

/// Decompress a spline-compressed animation, returning both transform frames
/// and float-track samples.
///
/// Float tracks share the per-block layout of transform tracks: their masks
/// live in the per-block mask-and-quant region after the 4 bytes per
/// transform track, and their payloads follow the transform-track payloads
/// inside the same block. Each track is a 1-component scalar track using the
/// same quantization type as track 0's translation channel (see writer
/// invariant in `compress_spline_with_params`).
pub fn decompress_spline_full(
    data: &[u8],
    num_tracks: u32,
    num_floats: u32,
    num_frames: u32,
    max_frames_per_block: u32,
    num_blocks: u32,
    block_offsets: &[u32],
    float_block_offsets: &[u32],
    mask_and_quant_size: u32,
    _block_duration: f32,
    block_inverse_duration: f32,
    frame_duration: f32,
) -> HavokResult<DecompressedSpline> {
    let mut all_frames: Vec<SplineFrame> = Vec::with_capacity(num_frames as usize);
    // float_tracks[track_idx] = Vec<f32> across all frames.
    let mut float_tracks: Vec<Vec<f32>> = (0..num_floats as usize)
        .map(|_| Vec::with_capacity(num_frames as usize))
        .collect();

    let stride = (max_frames_per_block as usize).saturating_sub(1).max(1);

    for frame_idx in 0..num_frames as usize {
        let block = (frame_idx / stride).min((num_blocks as usize).saturating_sub(1));
        let first_frame = block * stride;
        let local_frame = frame_idx - first_frame;
        let block_time = local_frame as f32 * frame_duration;

        // SDK (hkaSplineCompressedAnimation.inl:213-229 getBlockAndTime) casts to
        // hkUint8 which truncates. For valid data block_inverse_duration *
        // (max_frames_per_block - 1) ≤ 1/frame_duration so qt_f stays in [0,255]
        // except for possible FP rounding on the last frame; we clamp instead of
        // truncate/wrap — both yield 255 on SDK-produced data.
        let qt_f = block_time * block_inverse_duration * (max_frames_per_block as f32 - 1.0);
        let quantized_time = (qt_f as u32).min(255);

        let block_base = block_offsets.get(block).copied().ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "missing spline block offset {block} of {num_blocks}"
            ))
        })? as usize;
        let track_base = block_base
            .checked_add(mask_and_quant_size as usize)
            .ok_or_else(|| HavokError::InvalidInput("spline track offset overflow".into()))?;
        if block_base > data.len() || track_base > data.len() {
            return Err(HavokError::InvalidInput(format!(
                "spline block {block} is outside data (block={block_base}, tracks={track_base}, data_len={})",
                data.len()
            )));
        }

        let mut mask_r = Reader::new(data, block_base);
        let mut track_r = Reader::new(data, track_base);

        let mut transforms = Vec::with_capacity(num_tracks as usize);
        for _ti in 0..num_tracks as usize {
            let packed_q = mask_r.u8()?;
            let trans_mask = mask_r.u8()?;
            let rot_mask = mask_r.u8()?;
            let scale_mask = mask_r.u8()?;

            let trans_q = (packed_q & 0x03) as usize;
            let rot_q = ((packed_q >> 2) & 0x0F) as usize;
            let scale_q = ((packed_q >> 6) & 0x03) as usize;
            let translation = sample_scalar_track(
                &mut track_r,
                trans_q,
                quantized_time,
                frame_duration,
                block_time,
                trans_mask,
                [0.0, 0.0, 0.0],
            )?;

            let rotation = sample_rotation_track(
                &mut track_r,
                rot_q,
                quantized_time,
                frame_duration,
                block_time,
                rot_mask,
            )?;

            let scale = sample_scalar_track(
                &mut track_r,
                scale_q,
                quantized_time,
                frame_duration,
                block_time,
                scale_mask,
                [1.0, 1.0, 1.0],
            )?;

            transforms.push(SplineTransform {
                translation,
                rotation,
                scale,
            });
        }

        all_frames.push(SplineFrame { transforms });

        // Per-block float-track payloads. Mask bytes live immediately after
        // the per-track mask quartets (within mask_and_quant_size); payloads
        // immediately after the transform-track payloads in the same block.
        if num_floats > 0 {
            let float_payload_offset = float_block_offsets.get(block).copied().ok_or_else(|| {
                HavokError::InvalidInput(format!(
                    "missing float spline block offset {block} of {num_blocks}"
                ))
            })? as usize;
            let float_payload_base =
                block_base
                    .checked_add(float_payload_offset)
                    .ok_or_else(|| {
                        HavokError::InvalidInput("float spline payload offset overflow".into())
                    })?;
            if float_payload_base > data.len() {
                return Err(HavokError::InvalidInput(format!(
                    "float spline block {block} is outside data (payload={float_payload_base}, data_len={})",
                    data.len()
                )));
            }
            let mut float_r = Reader::new(data, float_payload_base);

            // Read float-track masks for this block.
            let mut float_masks: Vec<u8> = Vec::with_capacity(num_floats as usize);
            for _ in 0..num_floats {
                float_masks.push(mask_r.u8()?);
            }
            // sample_scalar_track advances float_r past each track's payload, so
            // every track is replayed and only this frame's value is kept.
            for (fi, raw_mask) in float_masks.iter().copied().enumerate() {
                let scalar_q = usize::from((raw_mask >> 1) & 1);
                let component_mask = raw_mask & 0b0001_0001;
                let v3 = sample_scalar_track(
                    &mut float_r,
                    scalar_q,
                    quantized_time,
                    frame_duration,
                    block_time,
                    component_mask,
                    [0.0, 0.0, 0.0],
                )?;
                if fi < float_tracks.len() {
                    float_tracks[fi].push(v3[0]);
                }
            }
        }
    }

    Ok(DecompressedSpline {
        frames: all_frames,
        float_tracks,
    })
}

// ---------------------------------------------------------------------------
// Spline compression
// ---------------------------------------------------------------------------

/// Spline compression knobs. `compress_spline` uses the defaults; call
/// `compress_spline_with_params` for e.g. POLAR32 rotations instead of THREECOMP40.
#[derive(Debug, Clone, Copy)]
pub struct SplineCompressionParams {
    pub rotation_type: RotationQuantization,
    pub scalar_type: ScalarQuantization,
    pub scale_type: ScalarQuantization,
    pub knot_tolerance: f32,
    pub translation_tolerance: f32,
    pub rotation_tolerance: f32,
    pub scale_tolerance: f32,
    pub max_frames_per_block: u32,
}

impl Default for SplineCompressionParams {
    fn default() -> Self {
        Self {
            rotation_type: RotationQuantization::ThreeComp40,
            scalar_type: ScalarQuantization::Bits16,
            scale_type: ScalarQuantization::Bits16,
            knot_tolerance: 0.001,
            translation_tolerance: 0.001,
            rotation_tolerance: 0.001,
            scale_tolerance: 0.001,
            max_frames_per_block: 256,
        }
    }
}

/// Output of `compress_spline`: the raw byte blob plus the metadata the
/// animation writer/decompressor needs.
#[derive(Debug, Clone)]
pub struct CompressedSplineBlob {
    /// The flat compressed byte buffer (all blocks concatenated).
    pub data: Vec<u8>,
    pub num_tracks: u32,
    pub num_floats: u32,
    pub num_frames: u32,
    pub num_blocks: u32,
    pub max_frames_per_block: u32,
    pub mask_and_quant_size: u32,
    pub block_duration: f32,
    pub block_inverse_duration: f32,
    pub frame_duration: f32,
    /// Byte offset of each block within `data`.
    pub block_offsets: Vec<u32>,
    /// Block-relative byte offset to each block's float-track payloads (one
    /// entry per block). Populated even with zero float tracks because FO4
    /// reads `floatBlockOffsets[block]` unconditionally while sampling.
    pub float_block_offsets: Vec<u32>,
}

/// Compress `frames[frame_idx].transforms[track_idx]` into a spline blob that
/// round-trips through `decompress_spline`. Fewer than 2 frames yields a static
/// blob of the first frame.
pub fn compress_spline(
    frames: &[SplineFrame],
    duration: f32,
    fps: f32,
) -> HavokResult<CompressedSplineBlob> {
    compress_spline_with_params(
        frames,
        &[],
        duration,
        fps,
        &SplineCompressionParams::default(),
    )
}

/// Like `compress_spline`, with custom params and optional float tracks
/// (`float_tracks[track_idx][frame_idx]`; pass `&[]` for none). Float tracks use
/// the vanilla per-block layout, so they round-trip through `decompress_spline_full`.
pub fn compress_spline_with_params(
    frames: &[SplineFrame],
    float_tracks: &[Vec<f32>],
    duration: f32,
    fps: f32,
    params: &SplineCompressionParams,
) -> HavokResult<CompressedSplineBlob> {
    let num_frames = frames.len() as u32;
    let num_tracks = frames
        .first()
        .map(|f| f.transforms.len() as u32)
        .unwrap_or(0);
    let num_floats = float_tracks.len() as u32;

    let frame_duration = if num_frames > 1 {
        duration / (num_frames - 1) as f32
    } else {
        if fps > 0.0 { 1.0 / fps } else { 1.0 / 30.0 }
    };

    let max_frames: u32 = params.max_frames_per_block;
    let num_blocks = ((num_frames + max_frames - 1) / max_frames).max(1);

    // block_duration matches Python: duration / (num_frames-1) * (max_frames-1)
    let block_duration = if num_frames > 1 {
        duration / (num_frames - 1) as f32 * (max_frames - 1) as f32
    } else {
        (max_frames - 1) as f32 * frame_duration
    };
    let block_inverse_duration = if block_duration > 0.0 {
        1.0 / block_duration
    } else {
        0.0
    };

    // mask_and_quant_size = align_up(4 * num_tracks + num_floats, 4)
    let mask_and_quant_size = align_up(4 * num_tracks + num_floats, 4);

    // Deinterleave: extract per-track, per-frame arrays
    let nf = num_frames as usize;
    let nt = num_tracks as usize;
    let mut track_pos: Vec<Vec<[f32; 3]>> = vec![Vec::with_capacity(nf); nt];
    let mut track_rot: Vec<Vec<[f32; 4]>> = vec![Vec::with_capacity(nf); nt];
    let mut track_scl: Vec<Vec<[f32; 3]>> = vec![Vec::with_capacity(nf); nt];

    for frame in frames {
        for (ti, xf) in frame.transforms.iter().enumerate() {
            if ti < nt {
                track_pos[ti].push(xf.translation);
                track_rot[ti].push(xf.rotation);
                track_scl[ti].push(xf.scale);
            }
        }
    }

    let rot_type: usize = params.rotation_type as usize;
    let trans_scalar_type: usize = params.scalar_type as usize;
    let scale_scalar_type: usize = params.scale_type as usize;
    let translation_tol: f32 = params.translation_tolerance;
    let rotation_tol: f32 = params.rotation_tolerance;
    let scale_tol: f32 = params.scale_tolerance;

    let mut data_buf: Vec<u8> = Vec::new();
    let mut block_offsets: Vec<u32> = Vec::with_capacity(num_blocks as usize);
    let mut float_block_offsets: Vec<u32> = Vec::with_capacity(num_blocks as usize);

    for block_idx in 0..num_blocks as usize {
        block_offsets.push(data_buf.len() as u32);

        let first_frame = block_idx * max_frames as usize;
        let last_frame_excl = ((block_idx + 1) * max_frames as usize).min(nf);
        let _block_frames = last_frame_excl - first_frame;

        // Reserve mask section (fill later)
        let mask_start = data_buf.len();
        data_buf.resize(mask_start + mask_and_quant_size as usize, 0u8);
        let mut mask_offset = mask_start;

        // Per-track data buffer (written separately then appended)
        let mut track_buf: Vec<u8> = Vec::new();

        for ti in 0..nt {
            let pos_slice = &track_pos[ti][first_frame..last_frame_excl];
            let rot_slice = &track_rot[ti][first_frame..last_frame_excl];
            let scl_slice = &track_scl[ti][first_frame..last_frame_excl];

            // Compute masks
            let pos_mask = compute_vec3_mask(pos_slice, translation_tol, [0.0, 0.0, 0.0]);
            let rot_mask = compute_rot_mask(rot_slice, rotation_tol);
            let scl_mask = compute_vec3_mask(scl_slice, scale_tol, [1.0, 1.0, 1.0]);

            // Pack quantization types byte:
            // bits 0-1 = translation scalar type, bits 2-5 = rotation type,
            // bits 6-7 = scale scalar type.
            let qt_byte: u8 = (trans_scalar_type as u8 & 0x03)
                | ((rot_type as u8 & 0x0F) << 2)
                | ((scale_scalar_type as u8 & 0x03) << 6);

            // Write mask bytes into reserved space
            data_buf[mask_offset] = qt_byte;
            data_buf[mask_offset + 1] = pos_mask;
            data_buf[mask_offset + 2] = rot_mask;
            data_buf[mask_offset + 3] = scl_mask;
            mask_offset += 4;

            // Write position track
            write_vec3_track(
                &mut track_buf,
                pos_slice,
                pos_mask,
                trans_scalar_type,
                [0.0, 0.0, 0.0],
            );

            // Write rotation track
            write_rot_track(&mut track_buf, rot_slice, rot_mask, rot_type);

            // Align to 4 after rotation (matches Python: data_buf.align(4) after rotation)
            align_buf(&mut track_buf, 4);

            // Write scale track
            write_vec3_track(
                &mut track_buf,
                scl_slice,
                scl_mask,
                scale_scalar_type,
                [1.0, 1.0, 1.0],
            );
        }

        // floatBlockOffsets[block] is the block-relative byte offset to the
        // float-track payloads — i.e. the end of this block's transform data
        // (mask region + all transform-track payloads). FO4 reads this entry
        // for every block *unconditionally*, regardless of float-track count
        // (hkaSplineCompressedAnimation.cpp:411 samplePartialTracks, :273
        // getDataChunks), so it must always be recorded. Emitting an empty
        // array deserializes to a null base in FO4 → null deref crash.
        float_block_offsets.push(mask_and_quant_size + track_buf.len() as u32);

        // Float-track masks: one byte per float track. Vanilla Havok stores
        // these immediately after the transform-track masks, before the
        // 4-byte alignment that introduces the per-track payload region.
        let mut float_masks: Vec<u8> = Vec::with_capacity(float_tracks.len());
        for ft in float_tracks {
            let block_samples: Vec<[f32; 3]> = ft[first_frame..last_frame_excl]
                .iter()
                .map(|v| [*v, 0.0, 0.0])
                .collect();
            let mask = compute_vec3_mask(&block_samples, translation_tol, [0.0, 0.0, 0.0]);
            // Only the low bits matter — 1-component track uses bit 0
            // (static low) and bit 4 (dynamic low). Mask out the upper
            // two component bits to keep the byte representation tidy.
            float_masks.push(mask & 0b0001_0001);
        }
        for (i, fm) in float_masks.iter().enumerate() {
            data_buf[mask_offset + i] = *fm | ((trans_scalar_type as u8 & 1) << 1);
        }

        // Float-track payloads. Each track is a 1-component scalar track,
        // so we re-use the same min/max/quantize machinery as vec3 by
        // padding to 3 components with zeros and only honouring component 0
        // in the mask.
        for (ft, fm) in float_tracks.iter().zip(float_masks.iter()) {
            let block_samples: Vec<[f32; 3]> = ft[first_frame..last_frame_excl]
                .iter()
                .map(|v| [*v, 0.0, 0.0])
                .collect();
            write_vec3_track(
                &mut track_buf,
                &block_samples,
                *fm,
                trans_scalar_type,
                [0.0, 0.0, 0.0],
            );
            align_buf(&mut track_buf, 4);
        }

        data_buf.extend_from_slice(&track_buf);

        // Align block to 16-byte boundary
        align_buf(&mut data_buf, 16);
    }

    Ok(CompressedSplineBlob {
        data: data_buf,
        num_tracks,
        num_floats,
        num_frames,
        num_blocks,
        max_frames_per_block: max_frames,
        mask_and_quant_size,
        block_duration,
        block_inverse_duration,
        frame_duration,
        block_offsets,
        float_block_offsets,
    })
}

// ---------------------------------------------------------------------------
// Quaternion packers (inverse of unpackers above)
// ---------------------------------------------------------------------------

/// Pack THREECOMP40: quaternion → 5 bytes (smallest-three, 12-bit).
pub fn pack_threecomp40(q: [f32; 4]) -> [u8; 5] {
    let q = normalize_quat(q);
    let abs_q = q.map(|c| c.abs());
    let maxi = abs_q
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(3);

    let w_sign: u8 = if q[maxi] < 0.0 { 1 } else { 0 };
    let kept: Vec<f32> = (0..4).filter(|&i| i != maxi).map(|i| q[i]).collect();

    // Quantize: map [-1/sqrt(2), 1/sqrt(2)] -> [0, 4095]
    let fractal: f32 = 0.000_345_436; // 1 / sqrt(2) / 2047
    let quantize = |v: f32| -> u32 {
        let q_val = (v / fractal + 2047.0).round() as i32;
        q_val.clamp(0, 4095) as u32
    };

    let a = quantize(kept[0]);
    let b = quantize(kept[1]);
    let c = quantize(kept[2]);

    // Pack into 5 bytes (40 bits)
    // a: bits 0..11, b: bits 12..23, c: bits 24..35, maxi: bits 36..37, w_sign: bit 38
    let raw: u64 = (a as u64)
        | ((b as u64) << 12)
        | ((c as u64) << 24)
        | ((maxi as u64) << 36)
        | ((w_sign as u64) << 38);

    [
        (raw & 0xFF) as u8,
        ((raw >> 8) & 0xFF) as u8,
        ((raw >> 16) & 0xFF) as u8,
        ((raw >> 24) & 0xFF) as u8,
        ((raw >> 32) & 0xFF) as u8,
    ]
}

/// Pack THREECOMP48: quaternion → 6 bytes (smallest-three, 15-bit).
pub fn pack_threecomp48(q: [f32; 4]) -> [u8; 6] {
    let q = normalize_quat(q);
    let abs_q = q.map(|c| c.abs());
    let maxi = abs_q
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(3);

    let w_sign: u16 = if q[maxi] < 0.0 { 1 } else { 0 };
    let kept: Vec<f32> = (0..4).filter(|&i| i != maxi).map(|i| q[i]).collect();

    let half: i32 = (1 << 14) - 1; // 16383
    let fractal: f32 = 0.000_043_161; // ~= 1/sqrt(2)/16383
    let mask15: i32 = (1 << 15) - 1;
    let quantize = |v: f32| -> u16 {
        let q_val = (v / fractal + half as f32).round() as i32;
        q_val.clamp(0, mask15) as u16
    };

    let va = quantize(kept[0]);
    let vb = quantize(kept[1]);
    let vc = quantize(kept[2]);

    // Encode shift bits into sign bits of first two int16s
    let sx: u16 = va | (((maxi & 1) as u16) << 15);
    let sy: u16 = vb | ((((maxi >> 1) & 1) as u16) << 15);
    let sz: u16 = vc | (w_sign << 15);

    let mut out = [0u8; 6];
    out[0..2].copy_from_slice(&sx.to_le_bytes());
    out[2..4].copy_from_slice(&sy.to_le_bytes());
    out[4..6].copy_from_slice(&sz.to_le_bytes());
    out
}

/// Pack POLAR32: quaternion → 4 bytes.
pub fn pack_polar32(q: [f32; 4]) -> [u8; 4] {
    let q = normalize_quat(q);
    let [x, y, z, w] = q;
    let pi = std::f32::consts::PI;
    let pi2 = pi * 0.5;
    let pi4 = pi2 * 0.5;

    let signs = [
        if x < 0.0 { 1u32 } else { 0 },
        if y < 0.0 { 1u32 } else { 0 },
        if z < 0.0 { 1u32 } else { 0 },
        if w < 0.0 { 1u32 } else { 0 },
    ];
    let (ax, ay, az, aw) = (x.abs(), y.abs(), z.abs(), w.abs());

    let r = aw;
    let magnitude = (ax * ax + ay * ay + az * az).sqrt();

    let r_mask: u32 = (1 << 10) - 1; // 1023
    let phi_frac = pi2 / 511.0;

    let r_clamped = r.clamp(0.0, 1.0);
    let r_raw = ((1.0 - r_clamped).max(0.0).sqrt() * r_mask as f32).round() as u32;
    let r_raw = r_raw.min(r_mask);

    let (phi, theta) = if magnitude > 1e-10 {
        let nz = (az / magnitude).clamp(-1.0, 1.0);
        let phi = nz.acos();
        let sin_phi = phi.sin();
        let theta = if sin_phi > 1e-10 {
            let nx = ax / magnitude;
            let ny = ay / magnitude;
            ny.atan2(nx)
        } else {
            0.0
        };
        (phi, theta)
    } else {
        (0.0, 0.0)
    };

    let phi_int = (phi / phi_frac).round() as u32;
    let phi_int = phi_int.min(511);

    let phi_theta = if phi_int > 0 {
        let theta_q = theta / pi4 * phi_int as f32;
        let theta_int = theta_q.round() as u32;
        let theta_int = theta_int.min(phi_int);
        phi_int * phi_int + theta_int
    } else {
        0
    };

    let phi_theta = phi_theta.min(0x3FFFF);

    let mut cval = phi_theta | (r_raw << 18);
    if signs[0] != 0 {
        cval |= 0x10000000;
    }
    if signs[1] != 0 {
        cval |= 0x20000000;
    }
    if signs[2] != 0 {
        cval |= 0x40000000;
    }
    if signs[3] != 0 {
        cval |= 0x80000000;
    }

    cval.to_le_bytes()
}

/// Pack THREECOMP24: quaternion → 3 bytes (smallest-three, 7-bit).
pub fn pack_threecomp24(q: [f32; 4]) -> [u8; 3] {
    let q = normalize_quat(q);
    let abs_q = q.map(|c| c.abs());
    let maxi = abs_q
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(3);

    let w_sign: u8 = if q[maxi] < 0.0 { 1 } else { 0 };
    let kept: Vec<f32> = (0..4).filter(|&i| i != maxi).map(|i| q[i]).collect();

    // Quantize: map [-1/sqrt(2), 1/sqrt(2)] -> [0, 127]
    // scale = 1 / (63 * sqrt(2)); center = 63
    let scale: f32 = 1.0 / (63.0 * std::f32::consts::SQRT_2);
    let quantize = |v: f32| -> u8 {
        let q_val = (v / scale + 63.0).round() as i32;
        q_val.clamp(0, 127) as u8
    };

    let a = quantize(kept[0]);
    let b = quantize(kept[1]);
    let c = quantize(kept[2]);

    // Encode: a[6:0], maxi_lsb in a[7]; b[6:0], (maxi>>1) in b[7]; c[6:0], w_sign in c[7]
    [
        a | ((maxi & 1) as u8) << 7,
        b | (((maxi >> 1) & 1) as u8) << 7,
        c | (w_sign << 7),
    ]
}

/// Pack UNCOMPRESSED: quaternion → 16 bytes (4 × f32 LE).
pub fn pack_uncompressed_quat(q: [f32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, &v) in q.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

// ---------------------------------------------------------------------------
// Scalar quantization packing
// ---------------------------------------------------------------------------

fn pack8(min: f32, max: f32, val: f32) -> u8 {
    if (max - min).abs() < f32::EPSILON {
        return 0;
    }
    let frac = (val - min) / (max - min);
    (frac * 255.0).round().clamp(0.0, 255.0) as u8
}

fn pack16(min: f32, max: f32, val: f32) -> u16 {
    if (max - min).abs() < f32::EPSILON {
        return 0;
    }
    let frac = (val - min) / (max - min);
    (frac * 65535.0).round().clamp(0.0, 65535.0) as u16
}

// ---------------------------------------------------------------------------
// Mask computation helpers
// ---------------------------------------------------------------------------

/// Compute a mask byte for a 3-component vector track.
/// Returns: DDDDSSSS where D=dynamic bits (high nibble), S=static bits (low nibble).
/// Each component i:
///   - IDENTITY: both bits 0
///   - STATIC: bit i set (low nibble)
///   - DYNAMIC: bit (i+4) set (high nibble)
fn compute_vec3_mask(samples: &[[f32; 3]], tol: f32, identity: [f32; 3]) -> u8 {
    if samples.is_empty() {
        return 0;
    }
    let n = samples.len() as f32;
    let mut mask: u8 = 0;
    for i in 0..3 {
        let values: Vec<f32> = samples.iter().map(|s| s[i]).collect();
        let mean = values.iter().sum::<f32>() / n;
        let min_v = values.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_v = values.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        if (mean - min_v).abs() <= tol && (mean - max_v).abs() <= tol {
            // Static
            if (mean - identity[i]).abs() > tol {
                mask |= 1 << i; // static non-identity
            }
        } else {
            // Dynamic
            mask |= 1 << (i + 4);
        }
    }
    mask
}

/// Compute rotation mask.
/// 0x00 = identity, 0x0F = static, 0xF0 = dynamic.
///
/// Pre-aligns sample hemispheres against samples[0] before averaging — without
/// this, a track that legitimately wraps through q ↔ -q averages to mean ≈ 0
/// and misclassifies as static-identity.
pub fn compute_rot_mask(samples: &[[f32; 4]], tol: f32) -> u8 {
    if samples.is_empty() {
        return 0;
    }
    if samples.len() == 1 {
        let q = samples[0];
        let identity = [0.0f32, 0.0, 0.0, 1.0];
        let diff: f32 = q
            .iter()
            .zip(identity.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        return if diff <= tol { 0 } else { 0x0F };
    }

    // Hemisphere-align: flip samples whose dot with samples[0] is negative.
    let s0 = samples[0];
    let aligned: Vec<[f32; 4]> = samples
        .iter()
        .map(|s| {
            let dot = s[0] * s0[0] + s[1] * s0[1] + s[2] * s0[2] + s[3] * s0[3];
            if dot < 0.0 {
                [-s[0], -s[1], -s[2], -s[3]]
            } else {
                *s
            }
        })
        .collect();

    // Mean quaternion (now safe).
    let n = aligned.len() as f32;
    let mut mean = [0.0f32; 4];
    for s in &aligned {
        for i in 0..4 {
            mean[i] += s[i];
        }
    }
    for i in 0..4 {
        mean[i] /= n;
    }
    let mean = normalize_quat(mean);

    // Check if all aligned samples are within tolerance of the mean
    let is_static = aligned.iter().all(|s| {
        let dot: f32 = s
            .iter()
            .zip(mean.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .abs()
            .clamp(0.0, 1.0);
        let angle_diff = 2.0 * dot.acos();
        angle_diff <= tol
    });

    if is_static {
        let identity = [0.0f32, 0.0, 0.0, 1.0];
        let dot: f32 = mean
            .iter()
            .zip(identity.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>()
            .abs()
            .clamp(0.0, 1.0);
        let angle_diff = 2.0 * dot.acos();
        if angle_diff <= tol { 0x00 } else { 0x0F }
    } else {
        0xF0
    }
}

// ---------------------------------------------------------------------------
// Track writing helpers (matching decompressor's read order)
// ---------------------------------------------------------------------------

/// Write a complete 3-component (position or scale) track.
/// Matches the read order in `sample_scalar_track`.
fn write_vec3_track(
    buf: &mut Vec<u8>,
    samples: &[[f32; 3]],
    mask: u8,
    scalar_type: usize,
    _identity: [f32; 3],
) {
    if mask == 0 {
        // Identity — nothing written, no alignment either (decompressor skips)
        return;
    }

    let has_dynamic = (mask & 0xF0) != 0;

    if has_dynamic {
        // Dynamic path: write knots, align 4, bounds, then quantized control points
        let num_cp = samples.len();
        let degree: usize = 1;
        let knots = build_uniform_knots(num_cp, degree);

        // Write knot header: u16 numItems (= num_cp - 1), u8 degree, u8[] knots
        let n = (num_cp - 1) as u16;
        buf.extend_from_slice(&n.to_le_bytes());
        buf.push(degree as u8);
        for k in &knots {
            buf.push(*k);
        }

        // Align 4
        align_buf(buf, 4);

        // Write bounds for each component
        let n_samples = samples.len() as f32;
        for i in 0..3 {
            if mask & (1 << (i + 4)) != 0 {
                let min_v = samples.iter().map(|s| s[i]).fold(f32::INFINITY, f32::min);
                let max_v = samples
                    .iter()
                    .map(|s| s[i])
                    .fold(f32::NEG_INFINITY, f32::max);
                buf.extend_from_slice(&min_v.to_le_bytes());
                buf.extend_from_slice(&max_v.to_le_bytes());
            } else if mask & (1 << i) != 0 {
                // Static component: write mean
                let mean: f32 = samples.iter().map(|s| s[i]).sum::<f32>() / n_samples;
                buf.extend_from_slice(&mean.to_le_bytes());
            }
        }

        // Quantized control points follow the f32 bounds directly. That position is
        // already 2-aligned, which is what the decompressor's align(2) expects.

        for cp in samples {
            for i in 0..3 {
                if mask & (1 << (i + 4)) != 0 {
                    let min_v = samples.iter().map(|s| s[i]).fold(f32::INFINITY, f32::min);
                    let max_v = samples
                        .iter()
                        .map(|s| s[i])
                        .fold(f32::NEG_INFINITY, f32::max);
                    if scalar_type == 0 {
                        // BITS8
                        buf.push(pack8(min_v, max_v, cp[i]));
                    } else {
                        // BITS16
                        let v = pack16(min_v, max_v, cp[i]);
                        buf.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }

        // Align 4 after control points
        align_buf(buf, 4);
    } else {
        // Static/identity only: align 4, write static floats, align 4
        align_buf(buf, 4);
        for i in 0..3 {
            if mask & (1 << i) != 0 {
                let n_s = samples.len() as f32;
                let mean: f32 = samples.iter().map(|s| s[i]).sum::<f32>() / n_s;
                buf.extend_from_slice(&mean.to_le_bytes());
            }
        }
        align_buf(buf, 4);
    }
}

/// Write a complete rotation track.
/// Matches the read order in `sample_rotation_track`.
fn write_rot_track(buf: &mut Vec<u8>, samples: &[[f32; 4]], mask: u8, rot_type: usize) {
    if mask == 0 {
        // Identity — nothing written
        return;
    }

    let has_dynamic = (mask & 0xF0) != 0;
    let has_static = (mask & 0x0F) != 0;
    let align = ROTATION_ALIGN[rot_type];
    let _size = ROTATION_SIZE[rot_type];

    if has_dynamic {
        let num_cp = samples.len();
        let degree: usize = 1;
        let knots = build_uniform_knots(num_cp, degree);

        // Write knot header
        let n = (num_cp - 1) as u16;
        buf.extend_from_slice(&n.to_le_bytes());
        buf.push(degree as u8);
        for k in &knots {
            buf.push(*k);
        }

        // Align before quaternion data
        if align > 1 {
            align_buf(buf, align);
        }

        // Write each control point quaternion
        for q in samples {
            let packed = pack_quat_by_type(rot_type, *q);
            buf.extend_from_slice(&packed);
        }
        // Note: the caller does align(4) after write_rot_track,
        // so we don't need to here.
    } else if has_static {
        // Compute mean quaternion
        let n = samples.len() as f32;
        let mut mean = [0.0f32; 4];
        for s in samples {
            for i in 0..4 {
                mean[i] += s[i];
            }
        }
        for i in 0..4 {
            mean[i] /= n;
        }
        let mean = normalize_quat(mean);

        // Align before static quaternion
        if align > 1 {
            align_buf(buf, align);
        }
        let packed = pack_quat_by_type(rot_type, mean);
        buf.extend_from_slice(&packed);
    }
}

/// Pack a quaternion using the specified rotation type.
fn pack_quat_by_type(rot_type: usize, q: [f32; 4]) -> Vec<u8> {
    let q = normalize_quat(q);
    match rot_type {
        0 => pack_polar32(q).to_vec(),
        1 => pack_threecomp40(q).to_vec(),
        2 => pack_threecomp48(q).to_vec(),
        3 => pack_threecomp24(q).to_vec(),
        4 => match pack_straight16_quat(q) {
            Ok(bytes) => bytes.to_vec(),
            Err(_) => pack_uncompressed_quat(q).to_vec(),
        },
        5 => pack_uncompressed_quat(q).to_vec(),
        _ => pack_uncompressed_quat(q).to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Knot builder
// ---------------------------------------------------------------------------

/// Build a clamped uniform knot vector for `num_frames` control points, degree `p`.
/// Returns a Vec<u8> of knot values (frame indices, 0..num_frames-1).
fn build_uniform_knots(num_frames: usize, degree: usize) -> Vec<u8> {
    let n = num_frames;
    let num_knots = n + degree + 1;
    let mut knots = Vec::with_capacity(num_knots);

    // First (degree+1) knots clamped to 0
    for _ in 0..=degree {
        knots.push(0u8);
    }

    // Interior knots uniformly spaced
    let num_interior = num_knots - 2 * (degree + 1);
    for i in 0..num_interior {
        let t = (i + 1) as f32 / (num_interior + 1) as f32 * (num_frames - 1) as f32;
        knots.push(t.round() as u8);
    }

    // Last (degree+1) knots clamped to num_frames-1
    for _ in 0..=degree {
        knots.push((num_frames - 1) as u8);
    }

    knots
}

// ---------------------------------------------------------------------------
// Alignment helpers
// ---------------------------------------------------------------------------

fn align_up(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

fn align_buf(buf: &mut Vec<u8>, alignment: usize) {
    let rem = buf.len() % alignment;
    if rem != 0 {
        let pad = alignment - rem;
        buf.resize(buf.len() + pad, 0u8);
    }
}

fn normalize_quat(q: [f32; 4]) -> [f32; 4] {
    let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if len > 1e-10 {
        q.map(|c| c / len)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn normalize(quat: [f32; 4]) -> HavokResult<[f32; 4]> {
    let length = quat.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length <= f32::EPSILON {
        return Err(HavokError::InvalidInput(
            "cannot normalize zero-length quaternion".to_string(),
        ));
    }
    Ok(quat.map(|value| value / length))
}

/// Reconstruct a quaternion from the "smallest-three" encoding.
fn smallest_three_reconstruct(vals: &[f32; 3], maxi: usize, sign_neg: bool) -> [f32; 4] {
    let mut q = [0.0f32; 4];
    let mut j = 0usize;
    for &v in vals {
        if j == maxi {
            j += 1;
        }
        q[j] = v;
        j += 1;
    }
    let sq: f32 = q.iter().map(|c| c * c).sum();
    q[maxi] = (1.0f32 - sq).max(0.0).sqrt();
    if sign_neg {
        q[maxi] = -q[maxi];
    }
    q
}

fn popcount_3(mask: u8) -> usize {
    ((mask & 1) + ((mask >> 1) & 1) + ((mask >> 2) & 1)) as usize
}

fn read_f32_le(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn ensure_len(data: &[u8], needed: usize, context: &str) -> HavokResult<()> {
    if data.len() < needed {
        return Err(HavokError::InvalidInput(format!(
            "{context} needs {needed} bytes, got {}",
            data.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_frames(num_tracks: usize, num_frames: usize) -> Vec<SplineFrame> {
        (0..num_frames)
            .map(|f| SplineFrame {
                transforms: (0..num_tracks)
                    .map(|t| {
                        let k = (f + t) as f32 * 0.01;
                        SplineTransform {
                            translation: [k, k * 2.0, k * 3.0],
                            rotation: [0.0, 0.0, (k * 0.1).sin(), (k * 0.1).cos()],
                            scale: [1.0, 1.0, 1.0],
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// FO4's `samplePartialTracks` (hkaSplineCompressedAnimation.cpp:411) and
    /// `getDataChunks` (cpp:273) index `m_floatBlockOffsets[block]` regardless of
    /// float-track count. An empty array deserializes to a null base and crashes,
    /// so there must be one offset per block even with zero float tracks.
    #[test]
    fn compress_spline_emits_one_float_block_offset_per_block_with_zero_floats() {
        let frames = ramp_frames(95, 4);
        let blob = compress_spline(&frames, 0.1, 30.0).expect("compress");

        assert_eq!(blob.num_floats, 0, "this fixture has no float tracks");
        assert_eq!(
            blob.float_block_offsets.len(),
            blob.num_blocks as usize,
            "floatBlockOffsets must have exactly num_blocks entries (got {}, num_blocks={})",
            blob.float_block_offsets.len(),
            blob.num_blocks,
        );

        // Each entry is block-relative: it must point past the mask region and
        // land within the data buffer when combined with the block's stream
        // offset (so FO4's `data + blockOffsets[b] + floatBlockOffsets[b]` is in
        // bounds).
        for b in 0..blob.num_blocks as usize {
            let rel = blob.float_block_offsets[b] as usize;
            assert!(
                rel >= blob.mask_and_quant_size as usize,
                "block {b}: float offset {rel} must be after the {}-byte mask region",
                blob.mask_and_quant_size,
            );
            let abs = blob.block_offsets[b] as usize + rel;
            assert!(
                abs <= blob.data.len(),
                "block {b}: float data start {abs} out of bounds (data len {})",
                blob.data.len(),
            );
        }
    }

    /// The per-block offset must be recorded for every block of a multi-block
    /// animation (frames spanning more than `maxFramesPerBlock`), each one
    /// block-relative and in bounds.
    #[test]
    fn compress_spline_float_block_offsets_are_per_block_and_in_bounds_multi_block() {
        let frames = ramp_frames(30, 10);
        let params = SplineCompressionParams {
            max_frames_per_block: 4,
            ..SplineCompressionParams::default()
        };
        let blob = compress_spline_with_params(&frames, &[], 0.3, 30.0, &params).expect("compress");

        assert!(blob.num_blocks >= 3, "fixture should span multiple blocks");
        assert_eq!(blob.float_block_offsets.len(), blob.num_blocks as usize);
        for b in 0..blob.num_blocks as usize {
            let rel = blob.float_block_offsets[b] as usize;
            assert!(rel >= blob.mask_and_quant_size as usize, "block {b}");
            assert!(
                blob.block_offsets[b] as usize + rel <= blob.data.len(),
                "block {b}"
            );
        }
    }
}
