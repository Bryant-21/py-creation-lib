// Mass-properties computation and serialization for FO4 hknpShapeMassProperties.
//
// Layout of the on-disk block (per hknpShapeMassProperties_0.xml +
// hkCompressedMassProperties_0.xml, total 0x30 bytes):
//
//   +0x00..0x10   hkReferencedObject parent (16 bytes, all zeros)
//   +0x10..0x18   centerOfMass: hkInt16[4]   (hkPackedVector3, 8 bytes)
//   +0x18..0x20   inertia:      hkInt16[4]   (hkPackedVector3, 8 bytes)
//   +0x20..0x28   majorAxisSpace: hkInt16[4] (hkPackedUnitVector<4>, 8 bytes)
//   +0x28..0x2C   mass:   hkReal (f32)
//   +0x2C..0x30   volume: hkReal (f32)
//
// Packing follows refs/hk2018_1_0_r1/Source/Common/Base/Math/Vector/hkPackedVector3.h.
// Without a source mass distribution, inertia is the AABB approximation
// (I = (1/12) m * (b² + c²) per principal axis).

/// Mass properties in real (uncompressed) units, ready for packing.
#[derive(Debug, Clone, PartialEq)]
pub struct MassProperties {
    pub mass: f32,
    pub volume: f32,
    /// Inverse-mass; 0.0 for static bodies (mass <= 0).
    pub inverse_mass: f32,
    /// Diagonal entries of the inverse inertia tensor (in major-axis-space).
    pub inverse_inertia_diag: [f32; 3],
    /// Center of mass in shape-local space.
    pub center_of_mass: [f32; 3],
    /// Quaternion (x, y, z, w) rotating from inertia major-axis-space to shape space.
    pub major_axis_space: [f32; 4],
}

impl MassProperties {
    pub fn zero() -> Self {
        MassProperties {
            mass: 0.0,
            volume: 0.0,
            inverse_mass: 0.0,
            inverse_inertia_diag: [0.0; 3],
            center_of_mass: [0.0; 3],
            major_axis_space: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Real (uncompressed) mass distribution decoded from a FO76 source body's
/// `hknpRefMassDistribution` (per the 2018 SDK `hknpMassDistribution` layout:
/// `centerOfMassAndVolume` vec4 with volume in w, `majorAxisSpace` quaternion,
/// `inertiaTensor` diagonalized for UNIT mass). All values are Havok units /
/// shape-local space. This is the object's *true* mass distribution — carrying
/// it replaces the AABB box approximation in [`polytope_mass_properties`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceMassDistribution {
    /// Center of mass in shape-local space (xyz).
    pub center_of_mass: [f32; 3],
    /// Shape volume (Havok units³). FO4 clutter is density 1.0, so mass == volume.
    pub volume: f32,
    /// Diagonalized inertia tensor for unit mass (multiply by mass for forward inertia).
    pub unit_inertia: [f32; 3],
    /// Quaternion (x,y,z,w): rotation from inertia major-axis space to shape space.
    pub major_axis_space: [f32; 4],
}

/// Build [`MassProperties`] from a FO76 source mass distribution at density 1.0
/// (FO4 clutter convention: shape mass == volume). COM, volume, and
/// majorAxisSpace are carried verbatim; forward inertia = `unit_inertia * mass`.
/// Falls back to zeroed (static) properties when the source volume is
/// non-positive or non-finite.
pub fn mass_properties_from_source(dist: &SourceMassDistribution) -> MassProperties {
    let mass = dist.volume;
    if mass <= 0.0 || !mass.is_finite() {
        return MassProperties::zero();
    }
    let inv = |i: f32| {
        if i > 0.0 && i.is_finite() {
            1.0 / i
        } else {
            0.0
        }
    };
    let forward = [
        dist.unit_inertia[0] * mass,
        dist.unit_inertia[1] * mass,
        dist.unit_inertia[2] * mass,
    ];
    let com = if dist.center_of_mass.iter().all(|c| c.is_finite()) {
        dist.center_of_mass
    } else {
        [0.0; 3]
    };
    let q = dist.major_axis_space;
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let major_axis_space = if n > 1e-6 && n.is_finite() {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    };
    MassProperties {
        mass,
        volume: mass,
        inverse_mass: 1.0 / mass,
        inverse_inertia_diag: [inv(forward[0]), inv(forward[1]), inv(forward[2])],
        center_of_mass: com,
        major_axis_space,
    }
}

/// AABB-approximated mass properties for a convex polytope: COM at the AABB
/// centroid, box inertia `I_x = (1/12) * mass * (extents_y^2 + extents_z^2)`
/// (and cyclic), identity major axes. Zeroed (static) for mass <= 0.
pub fn polytope_mass_properties(verts: &[[f32; 3]], mass: f32) -> MassProperties {
    if mass <= 0.0 || verts.is_empty() {
        return MassProperties::zero();
    }
    let (mn, mx) = verts.iter().fold(
        ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
        |(mn, mx), v| {
            (
                [mn[0].min(v[0]), mn[1].min(v[1]), mn[2].min(v[2])],
                [mx[0].max(v[0]), mx[1].max(v[1]), mx[2].max(v[2])],
            )
        },
    );
    let extents = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
    let center = [
        (mn[0] + mx[0]) * 0.5,
        (mn[1] + mx[1]) * 0.5,
        (mn[2] + mx[2]) * 0.5,
    ];
    let m12 = mass / 12.0;
    let ix = m12 * (extents[1] * extents[1] + extents[2] * extents[2]);
    let iy = m12 * (extents[0] * extents[0] + extents[2] * extents[2]);
    let iz = m12 * (extents[0] * extents[0] + extents[1] * extents[1]);
    let volume = extents[0] * extents[1] * extents[2];
    MassProperties {
        mass,
        volume,
        inverse_mass: 1.0 / mass,
        inverse_inertia_diag: [
            if ix > 0.0 { 1.0 / ix } else { 0.0 },
            if iy > 0.0 { 1.0 / iy } else { 0.0 },
            if iz > 0.0 { 1.0 / iz } else { 0.0 },
        ],
        center_of_mass: center,
        major_axis_space: [0.0, 0.0, 0.0, 1.0],
    }
}

/// AABB-approximated mass properties for a triangle mesh (usually static);
/// same as `polytope_mass_properties`, `_tris` is unused.
pub fn mesh_mass_properties(verts: &[[f32; 3]], _tris: &[[u32; 3]], mass: f32) -> MassProperties {
    polytope_mass_properties(verts, mass)
}

/// Compound mass properties from the union AABB of all children and the total
/// mass (per-child masses are ignored). Adequate for the static and kinematic
/// bodies that dominate FO4 vanilla content.
pub fn compound_mass_properties(
    children: &[(&[[f32; 3]], f32)],
    total_mass: f32,
) -> MassProperties {
    if total_mass <= 0.0 || children.is_empty() {
        return MassProperties::zero();
    }
    // Union AABB across all children, in compound-local space.
    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    let mut have_any = false;
    for (verts, _) in children {
        for v in *verts {
            mn[0] = mn[0].min(v[0]);
            mn[1] = mn[1].min(v[1]);
            mn[2] = mn[2].min(v[2]);
            mx[0] = mx[0].max(v[0]);
            mx[1] = mx[1].max(v[1]);
            mx[2] = mx[2].max(v[2]);
            have_any = true;
        }
    }
    if !have_any {
        return MassProperties::zero();
    }
    let extents = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
    let center = [
        (mn[0] + mx[0]) * 0.5,
        (mn[1] + mx[1]) * 0.5,
        (mn[2] + mx[2]) * 0.5,
    ];
    let m12 = total_mass / 12.0;
    let ix = m12 * (extents[1] * extents[1] + extents[2] * extents[2]);
    let iy = m12 * (extents[0] * extents[0] + extents[2] * extents[2]);
    let iz = m12 * (extents[0] * extents[0] + extents[1] * extents[1]);
    let volume = extents[0] * extents[1] * extents[2];
    MassProperties {
        mass: total_mass,
        volume,
        inverse_mass: 1.0 / total_mass,
        inverse_inertia_diag: [
            if ix > 0.0 { 1.0 / ix } else { 0.0 },
            if iy > 0.0 { 1.0 / iy } else { 0.0 },
            if iz > 0.0 { 1.0 / iz } else { 0.0 },
        ],
        center_of_mass: center,
        major_axis_space: [0.0, 0.0, 0.0, 1.0],
    }
}

// ---------------------------------------------------------------------------
// Inertia tensor diagonalization (Jacobi eigen-decomposition)
// ---------------------------------------------------------------------------

/// Diagonalize a symmetric 3×3 inertia tensor (row-major
/// `[Ixx, Ixy, Ixz, Ixy, Iyy, Iyz, Ixz, Iyz, Izz]`) with Jacobi rotations.
///
/// Returns `(eigenvalues, quaternion_xyzw)`; the quaternion rotates from
/// major-axis space into the body frame (`hknpMassDistribution::m_majorAxisSpace`).
/// Diagonal input yields the identity quaternion.
pub fn diagonalize_inertia(inertia_3x3: [f32; 9]) -> ([f32; 3], [f32; 4]) {
    // Working copies: a = symmetric matrix, v = accumulated rotation.
    let mut a: [[f64; 3]; 3] = [
        [
            inertia_3x3[0] as f64,
            inertia_3x3[1] as f64,
            inertia_3x3[2] as f64,
        ],
        [
            inertia_3x3[3] as f64,
            inertia_3x3[4] as f64,
            inertia_3x3[5] as f64,
        ],
        [
            inertia_3x3[6] as f64,
            inertia_3x3[7] as f64,
            inertia_3x3[8] as f64,
        ],
    ];
    let mut v: [[f64; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    const MAX_SWEEPS: usize = 50;
    const EPS: f64 = 1e-12;

    for _ in 0..MAX_SWEEPS {
        // Sum of squares of off-diagonals.
        let off = a[0][1].abs() + a[0][2].abs() + a[1][2].abs();
        if off < EPS {
            break;
        }
        // Sweep over the upper triangle.
        for p in 0..2 {
            for q in (p + 1)..3 {
                let apq = a[p][q];
                if apq.abs() < EPS {
                    continue;
                }
                let app = a[p][p];
                let aqq = a[q][q];
                // Compute Jacobi rotation angle.
                let theta = (aqq - app) / (2.0 * apq);
                let t = if theta.abs() > 1e15 {
                    1.0 / (2.0 * theta)
                } else {
                    let sign = if theta >= 0.0 { 1.0 } else { -1.0 };
                    sign / (theta.abs() + (theta * theta + 1.0).sqrt())
                };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;

                // Update matrix a: rotate rows/cols p and q.
                a[p][p] = app - t * apq;
                a[q][q] = aqq + t * apq;
                a[p][q] = 0.0;
                a[q][p] = 0.0;
                for i in 0..3 {
                    if i != p && i != q {
                        let aip = a[i][p];
                        let aiq = a[i][q];
                        a[i][p] = c * aip - s * aiq;
                        a[i][q] = s * aip + c * aiq;
                        a[p][i] = a[i][p];
                        a[q][i] = a[i][q];
                    }
                }
                // Accumulate eigenvectors in v.
                for i in 0..3 {
                    let vip = v[i][p];
                    let viq = v[i][q];
                    v[i][p] = c * vip - s * viq;
                    v[i][q] = s * vip + c * viq;
                }
            }
        }
    }

    // Ensure the eigenvector matrix is a proper rotation (det = +1).
    let det = v[0][0] * (v[1][1] * v[2][2] - v[1][2] * v[2][1])
        - v[0][1] * (v[1][0] * v[2][2] - v[1][2] * v[2][0])
        + v[0][2] * (v[1][0] * v[2][1] - v[1][1] * v[2][0]);
    if det < 0.0 {
        for i in 0..3 {
            v[i][0] = -v[i][0];
        }
    }

    // Convert rotation matrix v (columns are eigenvectors) to quaternion (xyzw).
    let trace = v[0][0] + v[1][1] + v[2][2];
    let (qx, qy, qz, qw) = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        let qw = 0.25 * s;
        let qx = (v[2][1] - v[1][2]) / s;
        let qy = (v[0][2] - v[2][0]) / s;
        let qz = (v[1][0] - v[0][1]) / s;
        (qx, qy, qz, qw)
    } else if v[0][0] > v[1][1] && v[0][0] > v[2][2] {
        let s = (1.0 + v[0][0] - v[1][1] - v[2][2]).sqrt() * 2.0;
        let qx = 0.25 * s;
        let qy = (v[0][1] + v[1][0]) / s;
        let qz = (v[0][2] + v[2][0]) / s;
        let qw = (v[2][1] - v[1][2]) / s;
        (qx, qy, qz, qw)
    } else if v[1][1] > v[2][2] {
        let s = (1.0 + v[1][1] - v[0][0] - v[2][2]).sqrt() * 2.0;
        let qx = (v[0][1] + v[1][0]) / s;
        let qy = 0.25 * s;
        let qz = (v[1][2] + v[2][1]) / s;
        let qw = (v[0][2] - v[2][0]) / s;
        (qx, qy, qz, qw)
    } else {
        let s = (1.0 + v[2][2] - v[0][0] - v[1][1]).sqrt() * 2.0;
        let qx = (v[0][2] + v[2][0]) / s;
        let qy = (v[1][2] + v[2][1]) / s;
        let qz = 0.25 * s;
        let qw = (v[1][0] - v[0][1]) / s;
        (qx, qy, qz, qw)
    };
    // Renormalize defensively (Jacobi accumulates rounding).
    let n = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt();
    let (qx, qy, qz, qw) = if n > 1e-12 {
        (qx / n, qy / n, qz / n, qw / n)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };

    (
        [a[0][0] as f32, a[1][1] as f32, a[2][2] as f32],
        [qx as f32, qy as f32, qz as f32, qw as f32],
    )
}

// ---------------------------------------------------------------------------
// Compressed encoding: hkPackedVector3 / hkPackedUnitVector<4>
// ---------------------------------------------------------------------------

/// Pack three f32 components into the 8-byte hkPackedVector3 representation.
///
/// hkPackedVector3 stores three signed 16-bit mantissas plus a shared scale word
/// `m3` that is the HIGH 16 bits of an IEEE-754 float32. The engine decodes it
/// (`hkPackedVector3::unpack`, SDK `Common/Base/Math/Vector/hkPackedVector3.h`)
/// as, per component:
///     value_i = (mantissa_i << 16 as f32) * bitcast_f32(m3 << 16)
///             = mantissa_i * 65536 * bitcast_f32(m3 << 16)
/// so the effective shared scale is `S = 65536 * bitcast_f32(m3 << 16)`.
///
/// The scale word `m3` MUST be a real float scale, not a raw power-of-2 exponent:
/// the engine reads it as `bitcast_f32(m3 << 16)`, so a tiny `m3` like 30 decodes
/// to a denormal ~2.7e-39, collapsing every nonzero-mass inertia to ~0 →
/// `inverseInertia = 1/0 = +Inf → NaN` → Havok AV on cell load. Vanilla inertia
/// scale words are ~10752 (=0x2A00, a normal float scale), never ~30.
///
/// Returns 8 bytes `[m0_lo, m0_hi, m1_lo, m1_hi, m2_lo, m2_hi, m3_lo, m3_hi]`.
pub fn pack_vector3(v: [f32; 3]) -> [u8; 8] {
    let max_abs = v[0].abs().max(v[1].abs()).max(v[2].abs());
    let mut out = [0u8; 8];
    if max_abs == 0.0 || !max_abs.is_finite() {
        // All-zero packed vector decodes to zero — also handles NaN/Inf and the
        // static-body case (inverse-of-zero inertia → +Inf), matching the
        // all-zero mass-props block vanilla writes for static (mass=0) shapes.
        return out;
    }
    // Pick the scale word m3 so every component fits in i16: we need
    // |v_i / S| <= 32767 with S = 65536 * bf and bf = bitcast_f32(m3 << 16)
    // (a float32 whose low 16 mantissa bits are zero). The smallest usable bf is
    // max_abs / (32767 * 65536); round its float bits UP to the next representable
    // high-16-bit value so the largest mantissa never overflows i16.
    let bf_min = max_abs / (32767.0 * 65536.0);
    let mut bits = bf_min.to_bits();
    if bits & 0x0000_FFFF != 0 {
        bits = (bits & 0xFFFF_0000).wrapping_add(0x0001_0000);
    }
    let m3 = (bits >> 16) as u16;
    let scale = 65536.0_f32 * f32::from_bits((m3 as u32) << 16);
    let to_i16 = |x: f32| -> i16 {
        let q = (x / scale).round();
        if q >= 32767.0 {
            32767
        } else if q <= -32768.0 {
            -32768
        } else {
            q as i16
        }
    };
    out[0..2].copy_from_slice(&to_i16(v[0]).to_le_bytes());
    out[2..4].copy_from_slice(&to_i16(v[1]).to_le_bytes());
    out[4..6].copy_from_slice(&to_i16(v[2]).to_le_bytes());
    out[6..8].copy_from_slice(&m3.to_le_bytes());
    out
}

/// Unpack hkPackedVector3 exactly as the engine does (`hkPackedVector3::unpack`).
pub fn unpack_vector3(bytes: &[u8; 8]) -> [f32; 3] {
    let m0 = i16::from_le_bytes([bytes[0], bytes[1]]) as f32;
    let m1 = i16::from_le_bytes([bytes[2], bytes[3]]) as f32;
    let m2 = i16::from_le_bytes([bytes[4], bytes[5]]) as f32;
    let m3 = u16::from_le_bytes([bytes[6], bytes[7]]);
    let scale = 65536.0_f32 * f32::from_bits((m3 as u32) << 16);
    [m0 * scale, m1 * scale, m2 * scale]
}

/// Pack a unit quaternion `(x, y, z, w)` into hkPackedUnitVector<4>.
///
/// hkPackedVector3.h:175 packs each component as the high 16 bits of
/// `(int32)(q * PACK16_UNIT_VEC) + 0x8000_0000`. With `setZero == 0x8000` this
/// reduces to `stored_u16 = trunc(q * 30000) + 32768` (scale derived from
/// vanilla clutter, whose identity majorAxisSpace is `[-32768,-32768,-32768,-2768]`
/// in i16: xyz == the zero value `0x8000`, w == +1). The engine normalizes after
/// unpack, so only the `+32768` offset must be exact; identity therefore
/// byte-matches vanilla. An offset/scale of 16384/16384 would decode to a
/// non-unit rotation, corrupting `R*diag(I)*R^T` into a NaN inertia tensor.
pub fn pack_unit_quat(q: [f32; 4]) -> [u8; 8] {
    let pack_one = |x: f32| -> u16 { ((x * 30000.0) as i32 + 32768).clamp(0, 65535) as u16 };
    let m0 = pack_one(q[0]);
    let m1 = pack_one(q[1]);
    let m2 = pack_one(q[2]);
    let m3 = pack_one(q[3]);
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&m0.to_le_bytes());
    out[2..4].copy_from_slice(&m1.to_le_bytes());
    out[4..6].copy_from_slice(&m2.to_le_bytes());
    out[6..8].copy_from_slice(&m3.to_le_bytes());
    out
}

/// Unpack hkPackedUnitVector<4> — inverse of `pack_unit_quat`, used by tests.
pub fn unpack_unit_quat(bytes: &[u8; 8]) -> [f32; 4] {
    let unp = |off: usize| -> f32 {
        let m = u16::from_le_bytes([bytes[off], bytes[off + 1]]) as i32;
        (m - 32768) as f32 / 30000.0
    };
    [unp(0), unp(2), unp(4), unp(6)]
}

// ---------------------------------------------------------------------------
// Block decompressor
// ---------------------------------------------------------------------------

/// Decompress a 0x30-byte `hknpShapeMassProperties` binary block into its
/// real-valued components.
///
/// Returns `Some((mass, forward_inertia, com_local, major_axis_space))`:
/// - `mass` — the body's total mass (kg). Always > 0 when `Some` is returned.
/// - `forward_inertia` — principal-axis diagonal of the **forward** inertia
///   tensor (not the inverse); decompressed from the `hkPackedVector3` at
///   `+0x18`. The caller must invert per-axis to obtain `inverseInertiaLocal`.
/// - `com_local` — center of mass in shape-local space; decompressed from
///   `+0x10`.
/// - `major_axis_space` — unit quaternion (x,y,z,w) rotating from inertia
///   major-axis space into shape space; decompressed from `+0x20`.
///
/// Returns `None` when the block's mass field (plain `f32` at `+0x28`) is
/// non-positive or non-finite (static or sentinel value).
pub fn decompress_mass_properties_block(
    bytes: &[u8; 0x30],
) -> Option<(f32, [f32; 3], [f32; 3], [f32; 4])> {
    let mass = f32::from_le_bytes(bytes[0x28..0x2C].try_into().unwrap());
    if !(mass > 0.0 && mass.is_finite()) {
        return None;
    }
    let com = unpack_vector3(bytes[0x10..0x18].try_into().unwrap());
    let forward_inertia = unpack_vector3(bytes[0x18..0x20].try_into().unwrap());
    let major_axis_space = unpack_unit_quat(bytes[0x20..0x28].try_into().unwrap());
    Some((mass, forward_inertia, com, major_axis_space))
}

// ---------------------------------------------------------------------------
// Block serializer
// ---------------------------------------------------------------------------

/// Serialize MassProperties into the 0x30-byte hknpShapeMassProperties block.
///
/// The first 16 bytes are the hkReferencedObject parent (zeroed).  The
/// remaining 32 bytes are hkCompressedMassProperties: packed centerOfMass,
/// packed inertia, packed majorAxisSpace, mass, volume.
pub fn serialize_mass_properties_block(mp: &MassProperties) -> [u8; 0x30] {
    let mut out = [0u8; 0x30];
    // +0x10..0x18: centerOfMass (hkPackedVector3)
    out[0x10..0x18].copy_from_slice(&pack_vector3(mp.center_of_mass));
    // +0x18..0x20: inertia (hkPackedVector3). hkCompressedMassProperties stores
    // the FORWARD principal inertia (the engine inverts it internally to get
    // inverseInertia); we hold the inverse diagonal, so convert back. A zero
    // inverse (static body) maps to forward 0 → all-zero packed block, matching
    // vanilla statics.
    let forward_inertia = [
        if mp.inverse_inertia_diag[0] > 0.0 {
            1.0 / mp.inverse_inertia_diag[0]
        } else {
            0.0
        },
        if mp.inverse_inertia_diag[1] > 0.0 {
            1.0 / mp.inverse_inertia_diag[1]
        } else {
            0.0
        },
        if mp.inverse_inertia_diag[2] > 0.0 {
            1.0 / mp.inverse_inertia_diag[2]
        } else {
            0.0
        },
    ];
    out[0x18..0x20].copy_from_slice(&pack_vector3(forward_inertia));
    // +0x20..0x28: majorAxisSpace (hkPackedUnitVector<4>)
    out[0x20..0x28].copy_from_slice(&pack_unit_quat(mp.major_axis_space));
    // +0x28..0x2C: mass (hkReal)
    out[0x28..0x2C].copy_from_slice(&mp.mass.to_le_bytes());
    // +0x2C..0x30: volume (hkReal)
    out[0x2C..0x30].copy_from_slice(&mp.volume.to_le_bytes());
    out
}

/// Verbatim compressed mass-properties block decoded from a source
/// `hknpShapeMassProperties`. The source exporter already packed COM / inertia /
/// majorAxisSpace with the hkPackedVector primitives, so the int16 words carry
/// straight through — no unpack/repack round trip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressedMassProperties {
    pub center_of_mass: [i16; 4],
    pub inertia: [i16; 4],
    pub major_axis_space: [i16; 4],
    pub mass: f32,
    pub volume: f32,
}

/// Serialize a verbatim [`CompressedMassProperties`] into the 0x30-byte
/// `hknpShapeMassProperties` block (same layout as
/// [`serialize_mass_properties_block`]).
pub fn serialize_compressed_mass_properties_block(props: &CompressedMassProperties) -> [u8; 0x30] {
    let mut out = [0u8; 0x30];
    for (i, v) in props.center_of_mass.iter().enumerate() {
        out[0x10 + i * 2..0x12 + i * 2].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in props.inertia.iter().enumerate() {
        out[0x18 + i * 2..0x1A + i * 2].copy_from_slice(&v.to_le_bytes());
    }
    for (i, v) in props.major_axis_space.iter().enumerate() {
        out[0x20 + i * 2..0x22 + i * 2].copy_from_slice(&v.to_le_bytes());
    }
    out[0x28..0x2C].copy_from_slice(&props.mass.to_le_bytes());
    out[0x2C..0x30].copy_from_slice(&props.volume.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_verts(half: f32) -> Vec<[f32; 3]> {
        vec![
            [-half, -half, -half],
            [half, -half, -half],
            [half, half, -half],
            [-half, half, -half],
            [-half, -half, half],
            [half, -half, half],
            [half, half, half],
            [-half, half, half],
        ]
    }

    #[test]
    fn polytope_mass_properties_and_serialized_block() {
        let static_mp = polytope_mass_properties(&cube_verts(1.0), 0.0);
        assert_eq!(static_mp.inverse_mass, 0.0);
        assert_eq!(static_mp.inverse_inertia_diag, [0.0; 3]);
        let zero_block = serialize_mass_properties_block(&MassProperties::zero());
        assert_eq!(zero_block.len(), 0x30);
        assert!(zero_block[0x00..0x10].iter().all(|b| *b == 0));
        assert_eq!(&zero_block[0x28..0x30], &[0u8; 8], "mass and volume zero");

        // 10 kg unit cube: I = (1/12)·10·(1+1) ≈ 1.667, inverse ≈ 0.6.
        let mp = polytope_mass_properties(&cube_verts(0.5), 10.0);
        assert!((mp.inverse_mass - 0.1).abs() < 1e-5);
        for c in mp.inverse_inertia_diag {
            assert!((c - 0.6).abs() < 1e-3, "inv inertia component = {c}");
        }
        let block = serialize_mass_properties_block(&mp);
        assert!((f32::from_le_bytes(block[0x28..0x2C].try_into().unwrap()) - 10.0).abs() < 1e-5);
        // The block stores the forward inertia, not the inverse held internally.
        for c in unpack_vector3(block[0x18..0x20].try_into().unwrap()) {
            assert!((c - 1.6667).abs() < 0.05, "forward inertia ~1.667, got {c}");
        }
    }

    #[test]
    fn packed_vector_and_quat_round_trip() {
        for v in [
            [0.0, 0.0, 0.0],
            [1.0, 2.0, 3.0],
            [-1.5, 0.5, -0.25],
            [100.0, -200.0, 300.0],
            [1e-3, 1e-4, 1e-5],
        ] {
            let unpacked = unpack_vector3(&pack_vector3(v));
            for i in 0..3 {
                // Shared exponent: small components relative to the dominant one
                // lose precision, so allow 2%.
                let rel_err = (v[i] - unpacked[i]).abs() / v[i].abs().max(1e-6);
                assert!(rel_err < 2e-2, "v={v:?} got {unpacked:?}");
            }
        }

        // Vanilla FO4 AlienToy inertia: mantissas [13200, 21797, 14798], scale word
        // 10752 decode to ~1e-4. The old raw power-of-2 exponent scheme decoded
        // them to ~1e-37 -> +Inf inverse inertia.
        let mut vanilla = [0u8; 8];
        vanilla[0..2].copy_from_slice(&13200i16.to_le_bytes());
        vanilla[2..4].copy_from_slice(&21797i16.to_le_bytes());
        vanilla[4..6].copy_from_slice(&14798i16.to_le_bytes());
        vanilla[6..8].copy_from_slice(&10752u16.to_le_bytes());
        for c in unpack_vector3(&vanilla) {
            assert!(c > 1e-6 && c < 1e-2, "vanilla inertia decoded to {c}");
        }
        let want = [9.8e-5_f32, 1.6e-4, 1.1e-4];
        let packed = pack_vector3(want);
        let scale_word = u16::from_le_bytes([packed[6], packed[7]]);
        assert!(
            scale_word > 4096,
            "scale word must be a float scale, got {scale_word}"
        );
        let back = unpack_vector3(&packed);
        for i in 0..3 {
            assert!((back[i] - want[i]).abs() / want[i] < 0.02);
        }

        let identity = pack_unit_quat([0.0, 0.0, 0.0, 1.0]);
        let words: Vec<i16> = identity
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(
            words,
            vec![-32768, -32768, -32768, -2768],
            "vanilla majorAxisSpace"
        );
        for q in [[0.0_f32, 0.0, 0.0, 1.0], [0.5, -0.5, 0.5, 0.5]] {
            let unpacked = unpack_unit_quat(&pack_unit_quat(q));
            for i in 0..4 {
                assert!((q[i] - unpacked[i]).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn mass_properties_from_source_distribution() {
        let dist = SourceMassDistribution {
            center_of_mass: [0.1, 0.2, -0.3],
            volume: 0.5,
            unit_inertia: [2.0, 4.0, 8.0],
            major_axis_space: [0.0, 0.0, 0.0, 1.0],
        };
        let mp = mass_properties_from_source(&dist);
        // Density 1.0: mass == volume; forward inertia = unit_inertia * mass.
        assert!((mp.mass - 0.5).abs() < 1e-6);
        assert!((mp.volume - 0.5).abs() < 1e-6);
        assert!((mp.inverse_mass - 2.0).abs() < 1e-6);
        assert_eq!(mp.center_of_mass, [0.1, 0.2, -0.3]);
        for (i, want) in [1.0, 0.5, 0.25].into_iter().enumerate() {
            assert!((mp.inverse_inertia_diag[i] - want).abs() < 1e-5);
        }
        let block = serialize_mass_properties_block(&mp);
        let com = unpack_vector3(block[0x10..0x18].try_into().unwrap());
        let inertia = unpack_vector3(block[0x18..0x20].try_into().unwrap());
        for (i, fwd) in [1.0, 2.0, 4.0].into_iter().enumerate() {
            assert!((com[i] - dist.center_of_mass[i]).abs() < 1e-3);
            assert!((inertia[i] - fwd).abs() / fwd < 0.02);
        }

        let zero_volume = mass_properties_from_source(&SourceMassDistribution {
            volume: 0.0,
            ..dist
        });
        assert_eq!(zero_volume.inverse_mass, 0.0);
        assert_eq!(zero_volume.inverse_inertia_diag, [0.0; 3]);
    }

    fn quat_norm(q: [f32; 4]) -> f32 {
        (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
    }

    fn quat_to_rot(q: [f32; 4]) -> [[f32; 3]; 3] {
        let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
        [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y - z * w),
                2.0 * (x * z + y * w),
            ],
            [
                2.0 * (x * y + z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z - x * w),
            ],
            [
                2.0 * (x * z - y * w),
                2.0 * (y * z + x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ]
    }

    fn matmul3(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
        std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
    }

    fn transpose3(a: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
        std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]))
    }

    #[test]
    fn diagonalize_inertia_recovers_principal_axes() {
        let sorted = |mut eigs: [f32; 3]| {
            eigs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            eigs
        };
        let (eigs, q) = diagonalize_inertia([0.0; 9]);
        assert_eq!(eigs, [0.0; 3]);
        assert!((quat_norm(q) - 1.0).abs() < 1e-5);

        let (eigs, q) = diagonalize_inertia([5.0, 0.0, 0.0, 0.0, 7.0, 0.0, 0.0, 0.0, 9.0]);
        for (got, want) in sorted(eigs).into_iter().zip([5.0, 7.0, 9.0]) {
            assert!((got - want).abs() < 1e-4);
        }
        assert!((quat_norm(q) - 1.0).abs() < 1e-5);

        // diag(2, 5, 8) rotated 30 degrees about Z.
        let (s, c) = 0.5235988_f32.sin_cos();
        let rot = [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]];
        let diag = [[2.0_f32, 0.0, 0.0], [0.0, 5.0, 0.0], [0.0, 0.0, 8.0]];
        let i_full = matmul3(matmul3(rot, diag), transpose3(rot));
        let (eigs, q) = diagonalize_inertia(std::array::from_fn(|k| i_full[k / 3][k % 3]));
        assert!((quat_norm(q) - 1.0).abs() < 1e-4);
        for (got, want) in sorted(eigs).into_iter().zip([2.0, 5.0, 8.0]) {
            assert!((got - want).abs() < 1e-3, "eig {got} vs {want}");
        }
        let r = quat_to_rot(q);
        let i_rot = matmul3(matmul3(transpose3(r), i_full), r);
        let off = i_rot[0][1].abs() + i_rot[0][2].abs() + i_rot[1][2].abs();
        assert!(off < 1e-3, "off-diagonals {off} in {i_rot:?}");
    }
}
