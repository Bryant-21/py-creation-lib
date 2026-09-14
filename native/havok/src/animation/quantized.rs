//! Reader for `hkaQuantizedAnimation` raw data buffers.
//!
//! Follows the SDK headers
//! (`refs/hk2018_1_0_r1/Source/Animation/Animation/Animation/Quantized/...`).
//!
//! ## Layout summary
//!
//! ```text
//! [Header (36 bytes)] [Static elements] [Static values] [Dynamic elements]
//! [Range minimums] [Range spans] [padding-to-16]
//! [Frame 0 dynamic values] [pad to frameSize multiples] ...
//! ```
//!
//! Static / dynamic each carry three categories: translations + scales (3D
//! scalar), rotations (48-bit smallest-three quaternion), floats (1D scalar).
//! Element indices encode bone/channel slot; values are either f32 (static) or
//! u16 quantized (dynamic, dequantized via `min + (val/65535) * span`).

use crate::error::{HavokError, HavokResult};

const HEADER_SIZE_BYTES: usize = 36;

/// Bit-for-bit mirror of `hkaQuantizedAnimation::QuantizedAnimationHeader`.
/// All fields are little-endian on x64. Keep ordering exact.
#[derive(Debug, Clone, Copy)]
pub struct QuantizedAnimationHeader {
    pub header_size: u16,

    pub num_bones: u16,
    pub num_floats: u16,
    pub num_frames: u16,
    pub duration: f32,

    pub num_static_translations: u16,
    pub num_static_rotations: u16,
    pub num_static_scales: u16,
    pub num_static_floats: u16,

    pub num_dynamic_translations: u16,
    pub num_dynamic_rotations: u16,
    pub num_dynamic_scales: u16,
    pub num_dynamic_floats: u16,

    pub frame_size: u16,

    pub static_elements_offset: u16,
    pub dynamic_elements_offset: u16,

    pub static_values_offset: u16,
    pub dynamic_range_minimums_offset: u16,
    pub dynamic_range_spans_offset: u16,
}

impl QuantizedAnimationHeader {
    fn read(bytes: &[u8]) -> HavokResult<Self> {
        if bytes.len() < HEADER_SIZE_BYTES {
            return Err(HavokError::InvalidInput(format!(
                "quantized header truncated: need {HEADER_SIZE_BYTES}, got {}",
                bytes.len()
            )));
        }
        let u16le = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        let f32le =
            |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        Ok(Self {
            header_size: u16le(0),
            num_bones: u16le(2),
            num_floats: u16le(4),
            num_frames: u16le(6),
            duration: f32le(8),
            num_static_translations: u16le(12),
            num_static_rotations: u16le(14),
            num_static_scales: u16le(16),
            num_static_floats: u16le(18),
            num_dynamic_translations: u16le(20),
            num_dynamic_rotations: u16le(22),
            num_dynamic_scales: u16le(24),
            num_dynamic_floats: u16le(26),
            frame_size: u16le(28),
            static_elements_offset: u16le(30),
            dynamic_elements_offset: u16le(32),
            static_values_offset: u16le(34),
            // Last 4 bytes (36-byte struct vs 40 fields read so far) read past
            // the 36-byte boundary. The SDK packs `m_dynamicRangeMinimumsOffset`
            // and `m_dynamicRangeSpansOffset` at offsets 36 and 38; require
            // 40-byte buffers when those are needed.
            dynamic_range_minimums_offset: 0,
            dynamic_range_spans_offset: 0,
        })
    }

    fn read_full(bytes: &[u8]) -> HavokResult<Self> {
        let mut hdr = Self::read(bytes)?;
        if bytes.len() < 40 {
            return Err(HavokError::InvalidInput(format!(
                "quantized header truncated: need 40 bytes for range offsets, got {}",
                bytes.len()
            )));
        }
        let u16le = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
        hdr.dynamic_range_minimums_offset = u16le(36);
        hdr.dynamic_range_spans_offset = u16le(38);
        Ok(hdr)
    }
}

/// One sampled pose: per-bone translation/rotation/scale, plus per-float value.
#[derive(Debug, Clone, Default)]
pub struct QuantizedPose {
    pub translations: Vec<[f32; 3]>,
    pub rotations: Vec<[f32; 4]>,
    pub scales: Vec<[f32; 3]>,
    pub floats: Vec<f32>,
}

/// Fully-decoded `hkaQuantizedAnimation`. Owns the raw blob plus the parsed
/// header so callers can re-sample at any time without re-parsing.
#[derive(Debug, Clone)]
pub struct QuantizedAnimation {
    pub header: QuantizedAnimationHeader,
    pub data: Vec<u8>,
}

impl QuantizedAnimation {
    /// Number of transform tracks (bones).
    pub fn num_tracks(&self) -> u32 {
        self.header.num_bones as u32
    }
    /// Number of original frames.
    pub fn num_frames(&self) -> u32 {
        self.header.num_frames as u32
    }
    /// Animation duration in seconds.
    pub fn duration(&self) -> f32 {
        self.header.duration
    }
    /// Frame stride in bytes (per-frame dynamic data block).
    pub fn frame_size(&self) -> usize {
        self.header.frame_size as usize
    }

    /// Sample the full pose at `time` seconds. Linear interpolation between
    /// adjacent frame samples; matches the `useSlerp = false` fast path of
    /// `hkaQuantizedAnimation::sampleFullPose`. SLERP can be added later if a
    /// caller needs strict SDK parity.
    pub fn sample_pose_at(&self, time: f32) -> HavokResult<QuantizedPose> {
        let (frame, delta) = self.frame_and_delta(time);
        let pose0 = self.sample_frame(frame)?;
        if delta <= 0.0 || frame + 1 >= self.header.num_frames as usize {
            return Ok(pose0);
        }
        let pose1 = self.sample_frame(frame + 1)?;
        Ok(blend_pose(&pose0, &pose1, delta))
    }

    fn frame_and_delta(&self, time: f32) -> (usize, f32) {
        let max_frame = (self.header.num_frames as i32 - 1).max(0);
        if max_frame <= 0 {
            return (0, 0.0);
        }
        let dur = self.header.duration.max(1e-12);
        let frame_f = (time / dur) * max_frame as f32;
        let frame_i = frame_f.floor() as i32;
        if frame_i >= max_frame {
            return ((max_frame - 1).max(0) as usize, 1.0);
        }
        if frame_i < 0 {
            return (0, 0.0);
        }
        let delta = frame_f - frame_i as f32;
        (frame_i as usize, delta.clamp(0.0, 1.0))
    }

    /// Sample a single frame index (no interpolation).
    pub fn sample_frame(&self, frame: usize) -> HavokResult<QuantizedPose> {
        if frame >= self.header.num_frames as usize {
            return Err(HavokError::InvalidInput(format!(
                "frame {frame} out of range (num_frames={})",
                self.header.num_frames
            )));
        }
        let mut pose = QuantizedPose {
            translations: vec![[0.0; 3]; self.header.num_bones as usize],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; self.header.num_bones as usize],
            scales: vec![[1.0, 1.0, 1.0]; self.header.num_bones as usize],
            floats: vec![0.0; self.header.num_floats as usize],
        };

        // Static elements live in the header region; same value every frame.
        self.apply_static(&mut pose)?;
        // Dynamic elements live in the per-frame block.
        let frame_offset =
            self.header.header_size as usize + frame * self.header.frame_size as usize;
        self.apply_dynamic(&mut pose, frame_offset)?;
        Ok(pose)
    }

    fn apply_static(&self, pose: &mut QuantizedPose) -> HavokResult<()> {
        let h = &self.header;
        let buf = &self.data;
        let mut elem_off = h.static_elements_offset as usize;
        let mut val_off = h.static_values_offset as usize;
        let n_trans_scale = h.num_static_translations as usize + h.num_static_scales as usize;
        // Translations + scales as f32 scalars indexed by element.
        write_static_scalars(pose, buf, elem_off, val_off, n_trans_scale)?;
        elem_off += n_trans_scale * 2;
        val_off += n_trans_scale * 4;
        // Rotations: per-quaternion 16-bit element + 3 × u16 packed rotation.
        write_static_rotations(
            pose,
            buf,
            elem_off,
            val_off,
            h.num_static_rotations as usize,
        )?;
        elem_off += h.num_static_rotations as usize * 2;
        val_off += h.num_static_rotations as usize * 6;
        // Floats: realign value offset to 16 (hkVector4 alignment) before reading.
        val_off = align_up(val_off, 16);
        write_static_floats(pose, buf, elem_off, val_off, h.num_static_floats as usize)?;
        Ok(())
    }

    fn apply_dynamic(&self, pose: &mut QuantizedPose, frame_offset: usize) -> HavokResult<()> {
        let h = &self.header;
        let buf = &self.data;
        let mut elem_off = h.dynamic_elements_offset as usize;
        let mut val_off = 0usize;
        let mut min_off = h.dynamic_range_minimums_offset as usize;
        let mut span_off = h.dynamic_range_spans_offset as usize;
        let n_trans_scale = h.num_dynamic_translations as usize + h.num_dynamic_scales as usize;
        write_dynamic_scalars(
            pose,
            buf,
            elem_off,
            min_off,
            span_off,
            frame_offset + val_off,
            n_trans_scale,
        )?;
        let off_short = n_trans_scale * 2;
        let off_real = n_trans_scale * 4;
        elem_off += off_short;
        val_off += off_short;
        min_off += off_real;
        span_off += off_real;
        // Dynamic rotations: per-quat 16-bit element, 3 × u16 packed value.
        write_dynamic_rotations(
            pose,
            buf,
            elem_off,
            frame_offset + val_off,
            h.num_dynamic_rotations as usize,
        )?;
        let off_short_rot = h.num_dynamic_rotations as usize * 2;
        elem_off += off_short_rot;
        val_off += off_short_rot * 3;
        // Floats: realign min/span to 16 (hkVector4 alignment) before reading.
        min_off = align_up(min_off, 16);
        span_off = align_up(span_off, 16);
        write_dynamic_scalars(
            pose,
            buf,
            elem_off,
            min_off,
            span_off,
            frame_offset + val_off,
            h.num_dynamic_floats as usize,
        )?;
        Ok(())
    }
}

/// Top-level API: read a quantized animation blob and parse the header.
pub fn read_quantized_animation(blob: &[u8]) -> HavokResult<QuantizedAnimation> {
    let header = QuantizedAnimationHeader::read_full(blob)?;
    Ok(QuantizedAnimation {
        header,
        data: blob.to_vec(),
    })
}

// ---------------------------------------------------------------------------
// Element category helpers
//
// Elements index a flat f32 stream with 12 floats per bone QsTransform:
// TX TY TZ TW QX QY QZ QW SX SY SZ SW, so bone = element / 12 and rotation
// starts at slot 4. (The SDK addresses rotations with stride 3, `element % 3 == 1`.)
// ---------------------------------------------------------------------------

const STRIDE_PER_BONE_FLOATS: usize = 12;

fn assign_scalar_to_pose(pose: &mut QuantizedPose, element: u16, value: f32, is_float_pass: bool) {
    let element = element as usize;
    if is_float_pass {
        if element < pose.floats.len() {
            pose.floats[element] = value;
        }
        return;
    }
    let stride = STRIDE_PER_BONE_FLOATS;
    let bone = element / stride;
    let slot = element % stride;
    if bone >= pose.translations.len() {
        return;
    }
    match slot {
        0..=2 => pose.translations[bone][slot] = value,
        // slots 3, 4, 5, 6, 7 (TW + Q components) handled by rotation pass.
        8..=10 => pose.scales[bone][slot - 8] = value,
        _ => {} // padding TW(3) / SW(11) / rotation slots — ignore
    }
}

fn write_static_scalars(
    pose: &mut QuantizedPose,
    buf: &[u8],
    elem_off: usize,
    val_off: usize,
    n: usize,
) -> HavokResult<()> {
    require_in_bounds(buf, elem_off, n * 2, "static scalar elements")?;
    require_in_bounds(buf, val_off, n * 4, "static scalar values")?;
    for i in 0..n {
        let element = u16le(buf, elem_off + i * 2);
        let value = f32le(buf, val_off + i * 4);
        assign_scalar_to_pose(pose, element, value, false);
    }
    Ok(())
}

fn write_static_floats(
    pose: &mut QuantizedPose,
    buf: &[u8],
    elem_off: usize,
    val_off: usize,
    n: usize,
) -> HavokResult<()> {
    require_in_bounds(buf, elem_off, n * 2, "static float elements")?;
    require_in_bounds(buf, val_off, n * 4, "static float values")?;
    for i in 0..n {
        let element = u16le(buf, elem_off + i * 2);
        let value = f32le(buf, val_off + i * 4);
        assign_scalar_to_pose(pose, element, value, true);
    }
    Ok(())
}

fn write_static_rotations(
    pose: &mut QuantizedPose,
    buf: &[u8],
    elem_off: usize,
    val_off: usize,
    n: usize,
) -> HavokResult<()> {
    require_in_bounds(buf, elem_off, n * 2, "static rotation elements")?;
    require_in_bounds(buf, val_off, n * 6, "static rotation values")?;
    for i in 0..n {
        let element = u16le(buf, elem_off + i * 2);
        let qq = [
            u16le(buf, val_off + i * 6),
            u16le(buf, val_off + i * 6 + 2),
            u16le(buf, val_off + i * 6 + 4),
        ];
        let q = unpack_quaternion_48_single(qq);
        // Element points to rotation slot of the bone's 12-stride QsTransform.
        let bone = element as usize / STRIDE_PER_BONE_FLOATS;
        if bone < pose.rotations.len() {
            pose.rotations[bone] = q;
        }
    }
    Ok(())
}

fn write_dynamic_scalars(
    pose: &mut QuantizedPose,
    buf: &[u8],
    elem_off: usize,
    min_off: usize,
    span_off: usize,
    val_off: usize,
    n: usize,
) -> HavokResult<()> {
    require_in_bounds(buf, elem_off, n * 2, "dynamic scalar elements")?;
    require_in_bounds(buf, min_off, n * 4, "dynamic minimums")?;
    require_in_bounds(buf, span_off, n * 4, "dynamic spans")?;
    require_in_bounds(buf, val_off, n * 2, "dynamic scalar values")?;
    for i in 0..n {
        let element = u16le(buf, elem_off + i * 2);
        let minimum = f32le(buf, min_off + i * 4);
        let span = f32le(buf, span_off + i * 4);
        let raw = u16le(buf, val_off + i * 2);
        let value = minimum + (raw as f32 / 65535.0) * span;
        // Route to the float array when the element index falls in the float
        // range (past the bone-stride slots) rather than a bone transform slot.
        let routed_to_float = (element as usize) < pose.floats.len()
            && (element as usize) >= pose.translations.len() * STRIDE_PER_BONE_FLOATS;
        assign_scalar_to_pose(pose, element, value, routed_to_float);
    }
    Ok(())
}

fn write_dynamic_rotations(
    pose: &mut QuantizedPose,
    buf: &[u8],
    elem_off: usize,
    val_off: usize,
    n: usize,
) -> HavokResult<()> {
    require_in_bounds(buf, elem_off, n * 2, "dynamic rotation elements")?;
    require_in_bounds(buf, val_off, n * 6, "dynamic rotation values")?;
    for i in 0..n {
        let element = u16le(buf, elem_off + i * 2);
        let qq = [
            u16le(buf, val_off + i * 6),
            u16le(buf, val_off + i * 6 + 2),
            u16le(buf, val_off + i * 6 + 4),
        ];
        let q = unpack_quaternion_48_single(qq);
        let bone = element as usize / STRIDE_PER_BONE_FLOATS;
        if bone < pose.rotations.len() {
            pose.rotations[bone] = q;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Quaternion 48-bit smallest-three unpack (single).
// Mirrors `hkaQuantizedQuaternion::unpackQuaternions48` semantics for one
// quaternion; we don't need the SIMD-batched version.
// ---------------------------------------------------------------------------

const SMALLEST_MIN: f32 = -0.7071067811865475_f32; // -sqrt(2)/2
const SMALLEST_RNG_INV: f32 = 4.316_100_721_397_47e-5_f32; // sqrt(2) / (2^15 - 2)

fn unpack_quaternion_48_single(qq: [u16; 3]) -> [f32; 4] {
    // Three small components, dequantized.
    let x = SMALLEST_MIN + ((qq[0] & 0x7FFF) as f32) * SMALLEST_RNG_INV;
    let y = SMALLEST_MIN + ((qq[1] & 0x7FFF) as f32) * SMALLEST_RNG_INV;
    let z = SMALLEST_MIN + ((qq[2] & 0x7FFF) as f32) * SMALLEST_RNG_INV;
    // Reconstruct the largest component (assume w = sqrt(1 - x² - y² - z²)).
    let mag_sq = x * x + y * y + z * z;
    let w_pos = (1.0_f32 - mag_sq).max(0.0).sqrt();
    // Sign of w from bit 15 of qq[2]: 1 - 2*(bit15) ∈ {1, -1}.
    let sign_bit = ((qq[2] >> 14) & 0x2) as i32;
    let w = w_pos * (1.0 - sign_bit as f32);
    // Largest-index = (qq[1].bit15 << 1) | qq[0].bit15.
    let largest = ((qq[1] >> 14) & 0x02) | ((qq[0] >> 15) & 0x01);
    // Build the (xyz, w) tuple, then shuffle the largest component (currently
    // at index 3 / "w" slot) into the largest-index slot.
    let mut q = [x, y, z, w];
    if largest != 3 {
        let li = (largest as usize).min(2);
        let tmp = q[li];
        q[li] = q[3];
        q[3] = tmp;
    }
    q
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

fn u16le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn f32le(buf: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn require_in_bounds(buf: &[u8], off: usize, len: usize, label: &str) -> HavokResult<()> {
    if off.saturating_add(len) > buf.len() {
        return Err(HavokError::InvalidInput(format!(
            "{label}: read past end (off={off}, len={len}, buf={})",
            buf.len()
        )));
    }
    Ok(())
}

fn blend_pose(a: &QuantizedPose, b: &QuantizedPose, t: f32) -> QuantizedPose {
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    let v3 = |xa: &[f32; 3], xb: &[f32; 3]| -> [f32; 3] {
        [lerp(xa[0], xb[0]), lerp(xa[1], xb[1]), lerp(xa[2], xb[2])]
    };
    let q = |qa: &[f32; 4], qb: &[f32; 4]| -> [f32; 4] {
        let dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
        let s = if dot < 0.0 { -1.0 } else { 1.0 };
        let raw = [
            lerp(qa[0], s * qb[0]),
            lerp(qa[1], s * qb[1]),
            lerp(qa[2], s * qb[2]),
            lerp(qa[3], s * qb[3]),
        ];
        let norm = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2] + raw[3] * raw[3]).sqrt();
        if norm > 1e-10 {
            [raw[0] / norm, raw[1] / norm, raw[2] / norm, raw[3] / norm]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    };
    QuantizedPose {
        translations: a
            .translations
            .iter()
            .zip(b.translations.iter())
            .map(|(x, y)| v3(x, y))
            .collect(),
        rotations: a
            .rotations
            .iter()
            .zip(b.rotations.iter())
            .map(|(x, y)| q(x, y))
            .collect(),
        scales: a
            .scales
            .iter()
            .zip(b.scales.iter())
            .map(|(x, y)| v3(x, y))
            .collect(),
        floats: a
            .floats
            .iter()
            .zip(b.floats.iter())
            .map(|(x, y)| lerp(*x, *y))
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Test fixture: a minimal static-only blob so tests need no FO76 file. Each
// bone gets static tx = index + 1, sx = 2.0 and an identity rotation; no
// floats, no dynamic data.
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub fn build_synthetic_static_blob(num_bones: u16, num_frames: u16, duration: f32) -> Vec<u8> {
    // Static-only layout:
    //  num_static_translations = num_bones (each bone gets a translation)
    //  num_static_rotations    = num_bones
    //  num_static_scales       = num_bones
    //  num_static_floats       = 0
    //  no dynamics
    let n = num_bones as usize;
    // Header section
    let mut hdr = vec![0u8; 40];
    let frame_size = 0u16;

    // Static elements: 3*n × u16 (translation slots + rotation slots + scale slots)
    // Layout per the SDK: translations + scales first, then rotations, then floats.
    let static_elements_off = 40u16;
    let n_trans_scale = (n + n) as usize; // num_static_translations + num_static_scales
    let n_rot = n;
    let static_elements_size = (n_trans_scale + n_rot) * 2;
    // Static values: one f32 per translation/scale element (the x slot of each
    // bone), then 3 × u16 = 6 bytes per rotation.
    let static_values_off = (static_elements_off as usize + static_elements_size) as u16;
    let static_values_trans_scale_size = n_trans_scale * 4;
    let static_values_rot_size = n_rot * 6;

    // Pack the synthetic data.
    // Bone elements: for translation slot 0 of bone b → element = b*12 + 0.
    // For scale slot 0 of bone b → element = b*12 + 8.
    // For rotation of bone b → element = b*12 + 4 (rotation x slot — but we
    // map by bone via element/12, so any slot in [4,7] works).
    let mut elems: Vec<u8> = Vec::new();
    for b in 0..num_bones {
        let e = (b as u16) * STRIDE_PER_BONE_FLOATS as u16; // translation x
        elems.extend_from_slice(&e.to_le_bytes());
    }
    for b in 0..num_bones {
        let e = (b as u16) * STRIDE_PER_BONE_FLOATS as u16 + 8; // scale x
        elems.extend_from_slice(&e.to_le_bytes());
    }
    for b in 0..num_bones {
        let e = (b as u16) * STRIDE_PER_BONE_FLOATS as u16 + 4; // rotation slot
        elems.extend_from_slice(&e.to_le_bytes());
    }

    let mut vals: Vec<u8> = Vec::new();
    // n translations: each bone's tx = (b as f32 + 1.0) for testability.
    for b in 0..num_bones {
        vals.extend_from_slice(&((b as f32 + 1.0).to_le_bytes()));
    }
    // n scales: each bone's sx = 2.0 for testability.
    for _ in 0..num_bones {
        vals.extend_from_slice(&2.0_f32.to_le_bytes());
    }
    // n rotations: identity packed via SDK 48-bit format.
    let identity_packed = pack_identity_quat_48();
    for _ in 0..num_bones {
        vals.extend_from_slice(&identity_packed);
    }

    // Compose blob: header(40) + elements + values. Compute frame_size = 0 (no dyn).
    // Dynamic offsets: point to end of values (no dynamic content).
    let dynamic_elements_off = (static_values_off as usize
        + static_values_trans_scale_size
        + static_values_rot_size) as u16;
    let dynamic_minimums_off = dynamic_elements_off;
    let dynamic_spans_off = dynamic_elements_off;

    // Fill header fields (offsets 0..40).
    let header_size = 40u16;
    hdr[0..2].copy_from_slice(&header_size.to_le_bytes());
    hdr[2..4].copy_from_slice(&num_bones.to_le_bytes());
    hdr[4..6].copy_from_slice(&0u16.to_le_bytes()); // num_floats
    hdr[6..8].copy_from_slice(&num_frames.to_le_bytes());
    hdr[8..12].copy_from_slice(&duration.to_le_bytes());
    hdr[12..14].copy_from_slice(&num_bones.to_le_bytes()); // num_static_translations
    hdr[14..16].copy_from_slice(&num_bones.to_le_bytes()); // num_static_rotations
    hdr[16..18].copy_from_slice(&num_bones.to_le_bytes()); // num_static_scales
    hdr[18..20].copy_from_slice(&0u16.to_le_bytes()); // num_static_floats
    hdr[20..22].copy_from_slice(&0u16.to_le_bytes()); // num_dynamic_translations
    hdr[22..24].copy_from_slice(&0u16.to_le_bytes()); // num_dynamic_rotations
    hdr[24..26].copy_from_slice(&0u16.to_le_bytes()); // num_dynamic_scales
    hdr[26..28].copy_from_slice(&0u16.to_le_bytes()); // num_dynamic_floats
    hdr[28..30].copy_from_slice(&frame_size.to_le_bytes());
    hdr[30..32].copy_from_slice(&static_elements_off.to_le_bytes());
    hdr[32..34].copy_from_slice(&dynamic_elements_off.to_le_bytes());
    hdr[34..36].copy_from_slice(&static_values_off.to_le_bytes());
    hdr[36..38].copy_from_slice(&dynamic_minimums_off.to_le_bytes());
    hdr[38..40].copy_from_slice(&dynamic_spans_off.to_le_bytes());

    let mut blob = Vec::with_capacity(40 + elems.len() + vals.len());
    blob.extend_from_slice(&hdr);
    blob.extend_from_slice(&elems);
    blob.extend_from_slice(&vals);
    blob
}

/// Pack identity quaternion (0,0,0,1) into the 48-bit smallest-three layout.
/// The largest is W (index 3), so:
///  - largest-index bits: high bits of qq[0] = bit0 of largest (1), high bit
///    of qq[1] (bit15) = bit1 of largest (1) → qq[1] |= 0x8000, qq[0] |= 0x8000
///  - W is positive → sign bit of qq[2] (bit15) = 0
///  - x=y=z=0 → q_value = (0 - SMALLEST_MIN) / SMALLEST_RNG_INV = sqrt(2)/2 / SMALLEST_RNG_INV
fn pack_identity_quat_48() -> [u8; 6] {
    // Quantize x = y = z = 0 to a 15-bit u16.
    let q_zero = ((0.0_f32 - SMALLEST_MIN) / SMALLEST_RNG_INV).round() as u16 & 0x7FFF;
    // Largest-index = 3 → bit 0 set (qq[0]>>15 = 1) and bit 1 set (qq[1]>>14 & 0x2 = 2 → qq[1] bit 15 = 1).
    let qq0 = q_zero | 0x8000;
    let qq1 = q_zero | 0x8000;
    let qq2 = q_zero; // sign bit (bit 15) = 0 → positive W
    let mut out = [0u8; 6];
    out[0..2].copy_from_slice(&qq0.to_le_bytes());
    out[2..4].copy_from_slice(&qq1.to_le_bytes());
    out[4..6].copy_from_slice(&qq2.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_synthetic_blob() {
        let blob = build_synthetic_static_blob(2, 3, 1.0);
        let anim = read_quantized_animation(&blob).expect("parse");
        assert_eq!(anim.num_tracks(), 2);
        assert_eq!(anim.num_frames(), 3);
        assert!((anim.duration() - 1.0).abs() < 1e-6);
        assert_eq!(anim.header.num_static_translations, 2);
        assert_eq!(anim.header.num_static_rotations, 2);
        assert_eq!(anim.header.num_static_scales, 2);
    }

    #[test]
    fn quantized_animation_reads_full_pose_from_synthetic_static_blob() {
        let blob = build_synthetic_static_blob(2, 3, 1.0);
        let anim = read_quantized_animation(&blob).expect("parse");
        assert!(anim.num_tracks() > 0);
        assert!(anim.num_frames() > 1);
        let pose0 = anim.sample_pose_at(0.0).expect("sample");
        assert_eq!(pose0.translations.len(), anim.num_tracks() as usize);
        // Bone 0: tx = 1.0; bone 1: tx = 2.0 (per build_synthetic_static_blob).
        assert!((pose0.translations[0][0] - 1.0).abs() < 1e-4);
        assert!((pose0.translations[1][0] - 2.0).abs() < 1e-4);
        // All scales sx = 2.0.
        assert!((pose0.scales[0][0] - 2.0).abs() < 1e-4);
        // Identity rotation: w ≈ 1, others ≈ 0.
        let q = pose0.rotations[0];
        assert!(q[3].abs() > 0.99, "expected w≈±1, got {q:?}");
        assert!(
            q[0].abs() < 0.01 && q[1].abs() < 0.01 && q[2].abs() < 0.01,
            "expected identity rotation, got {q:?}"
        );
    }

    #[test]
    fn unpack_quaternion_48_identity_round_trips() {
        let packed = pack_identity_quat_48();
        let qq = [
            u16::from_le_bytes([packed[0], packed[1]]),
            u16::from_le_bytes([packed[2], packed[3]]),
            u16::from_le_bytes([packed[4], packed[5]]),
        ];
        let q = unpack_quaternion_48_single(qq);
        // Identity (0,0,0,1) — w must be ≈1 and the others ≈0 (within
        // smallest-three quantization tolerance ~1e-4).
        assert!(q[3].abs() > 0.99, "w slot wrong: {q:?}");
        for i in 0..3 {
            assert!(
                q[i].abs() < 0.01,
                "component {i} should be ~0: got {}",
                q[i]
            );
        }
    }

    #[test]
    fn frame_and_delta_at_endpoints() {
        let blob = build_synthetic_static_blob(1, 4, 1.0);
        let anim = read_quantized_animation(&blob).unwrap();
        let (f0, d0) = anim.frame_and_delta(0.0);
        assert_eq!(f0, 0);
        assert!(d0 < 1e-4);
        let (fend, dend) = anim.frame_and_delta(1.0);
        assert!(fend == 2 || fend == 3);
        assert!(dend >= 0.0 && dend <= 1.0);
    }
}
