//! AnimTextData bucket body writers (CK-free).
//!
//! Filenames are byte-identical to CK (the engine opens them by name) and the content
//! format is byte-exact, but content-list order is ours: CK's order is an hkbHkxDB
//! artifact, not reproducible offline, and the engine reads the body into a lookup, so
//! order does not change which animations play. See `docs/re/animtextdata_generation.md`.

use std::collections::HashSet;

/// Build the `AnimationFileData/<id>.txt` body bytes.
///
/// `files` are full FO4 animation paths (e.g. `Actors\X\Animations\Y.hkx`), emitted in
/// order. CK format: `"3\n" "1\n" "<id>\n" "<count>\n"`, then each file plus `'\n'`
/// (including after the last).
///
/// Entries are deduplicated case-insensitively, keeping the first spelling: no vanilla
/// file lists a path twice (969690 entries across 3759 shipped files), but graph closure
/// and the on-disk SAPT sweep can reach the same clip with different casing. `count` is
/// the deduplicated length.
pub fn animation_file_data_body(id: u64, files: &[String]) -> Vec<u8> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let unique: Vec<&String> = files
        .iter()
        .filter(|f| seen.insert(f.replace('/', "\\").to_ascii_lowercase()))
        .collect();

    let mut s = String::with_capacity(48 + unique.len() * 48);
    s.push_str("3\n1\n"); // version, flag (1 = hash-named subgraph)
    s.push_str(&id.to_string());
    s.push('\n');
    s.push_str(&unique.len().to_string());
    s.push('\n');
    for f in unique {
        s.push_str(f);
        s.push('\n');
    }
    s.into_bytes()
}

/// One `AnimEventInfo` event: an animation event name, an integer flag (the middle
/// field, 0 for every observed Snallygaster event), and the clip names it maps to.
pub struct AnimEvent {
    pub name: String,
    pub flag: u32,
    pub clips: Vec<String>,
}

// ===========================================================================
// Shared binary primitive (ClipGeneratorData / AnimationOffsets / StanceData)
// ===========================================================================

/// Append a length-prefixed, NUL-terminated Pascal string. The length byte
/// **includes** the trailing NUL (`strlen + 1`); an empty string is a single
/// `0x00` length byte with no body. Packed/unaligned — numeric fields that
/// follow are written immediately after. (RE: `binary_AnimationOffsets.md`.)
fn push_pstr(out: &mut Vec<u8>, s: &str) {
    if s.is_empty() {
        out.push(0);
        return;
    }
    let bytes = s.as_bytes();
    // CK stores the length in a single byte; every load-bearing path/name is far
    // under 254 chars. Guard so a pathological input fails loudly in debug.
    debug_assert!(bytes.len() < 255, "pstr too long for u8 length: {s:?}");
    out.push((bytes.len() + 1) as u8);
    out.extend_from_slice(bytes);
    out.push(0);
}

// ===========================================================================
// SyncAnimData (text V4) — converted creatures have no paired anims
// ===========================================================================

/// Body for `SyncAnimData/ResolvedSyncAnimData<ProjectName>.txt`.
///
/// FO76→FO4 converted creatures author no paired/synchronised animations, so the
/// clean empty form is emitted verbatim — `b"V4\n0\n"` (G=0). The converted
/// B21_Snallygaster oracle is exactly these 5 bytes. (RE: `text_syncanimdata.md`.)
pub fn sync_anim_data_body() -> Vec<u8> {
    b"V4\n0\n".to_vec()
}

/// Body for the **existing-project** form — group count `G=1` (`b"V4\n1\n"`).
///
/// CK writes one empty group header (no entries) when it loads a real project from disk
/// that has no synchronized anims. Every FO76 creature ships a real `project.hkx`, so the
/// un-prefixed `ResolvedSyncAnimData<race>.txt` is always this form; the mod-prefixed
/// identity is a synthesized project with no `project.hkx` → `G=0` ([`sync_anim_data_body`]).
/// Both have zero sync entries; only `G` differs. (RE: `syncanim_count.md`.)
pub fn sync_anim_data_body_existing() -> Vec<u8> {
    b"V4\n1\n".to_vec()
}

// ===========================================================================
// Named project manifest (AnimationFileData/<projectname>.txt, flag 0)
// ===========================================================================

/// Body for the flag-0 named project manifest in the `AnimationFileData/` bucket dir,
/// next to the numeric `<id>.txt` files.
///
/// Versus [`animation_file_data_body`] (RE: `text_project_manifests.md`): flag line `0`,
/// a project-name line instead of the decimal id, CRLF line endings instead of LF, and a
/// trailing `0` line after the file list. `files` are project-relative
/// (`Behaviors\…RootBehavior.hkx`, `Animations\Idle.hkx`).
pub fn project_manifest_body(project_name: &str, files: &[String]) -> Vec<u8> {
    let mut s = String::with_capacity(32 + files.len() * 40);
    s.push_str("3\r\n0\r\n");
    s.push_str(project_name);
    s.push_str("\r\n");
    s.push_str(&files.len().to_string());
    s.push_str("\r\n");
    for f in files {
        s.push_str(f);
        s.push_str("\r\n");
    }
    s.push_str("0\r\n"); // trailing second count (0 in every observed sample)
    s.into_bytes()
}

// ===========================================================================
// ClipGeneratorData (binary, despite .txt)
// ===========================================================================

/// One `hkbClipTrigger` of a clip: the event name, its (signed) local time, and a
/// flag byte (nonzero means the engine evaluates `clipDuration - time`).
pub struct ClipTrigger {
    pub name: String,
    pub time: f32,
    pub flag: u8,
}

/// One `ClipGeneratorData` entry = one `hkbClipGenerator`.
pub struct ClipGenEntry {
    pub clip_name: String,
    /// Basename (no ext) of `animationName`; **empty** for dynamic clips.
    pub anim_name: String,
    pub playback_speed: f32,
    pub crop_start: f32,
    pub crop_end: f32,
    /// `x0` — 0 in every observed entry.
    pub x0: u8,
    /// `x1` — set for clips driven by a `DynamicAnim*`/`*TaggingGenerator`.
    pub dynamic: bool,
    pub triggers: Vec<ClipTrigger>,
}

/// Build `ClipGeneratorData/<name_id>.txt` body bytes (binary V4).
///
/// Format (RE: `text_clipgeneratordata.md`, round-trips byte-identical on the
/// Snallygaster oracles): `"V4\n"`, `pstr(behaviorPath)`, `u32(count)`, then per
/// entry `pstr(clip) pstr(anim) f32 f32 f32 u8 u8 u32(nTrig)` and per trigger
/// `pstr(name) f32(time) u8(flag)`. No trailing padding. CK's entry count/order is a
/// behavior-arena traversal artifact and is not reproduced; the byte format is exact.
pub fn clip_generator_data_body(behavior_path: &str, entries: &[ClipGenEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + behavior_path.len() + entries.len() * 32);
    out.extend_from_slice(b"V4\n");
    push_pstr(&mut out, behavior_path);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        push_pstr(&mut out, &e.clip_name);
        push_pstr(&mut out, &e.anim_name);
        out.extend_from_slice(&e.playback_speed.to_le_bytes());
        out.extend_from_slice(&e.crop_start.to_le_bytes());
        out.extend_from_slice(&e.crop_end.to_le_bytes());
        out.push(e.x0);
        out.push(u8::from(e.dynamic));
        out.extend_from_slice(&(e.triggers.len() as u32).to_le_bytes());
        for t in &e.triggers {
            push_pstr(&mut out, &t.name);
            out.extend_from_slice(&t.time.to_le_bytes());
            out.push(t.flag);
        }
    }
    out
}

// ===========================================================================
// AnimationOffsets (binary, despite .txt)
// ===========================================================================

/// Build the empty `AnimationOffsets/<id>.txt` body: `"V4\n"`, `pstr(core_behavior)`,
/// `u32(0)` (section-1 count), `u32(0)` (section-2 count).
///
/// Emitted only for behaviors with no root-motion/offset data, such as the ROOT behavior
/// (`1776463414.txt`, RE: `binary_AnimationOffsets.md`). Never for a subgraph with motion:
/// the engine trusts a present-but-empty cache and suppresses root motion instead of
/// rebuilding it.
pub fn animation_offsets_empty_body(core_behavior: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + core_behavior.len());
    out.extend_from_slice(b"V4\n");
    push_pstr(&mut out, core_behavior);
    out.extend_from_slice(&0u32.to_le_bytes()); // section1 count
    out.extend_from_slice(&0u32.to_le_bytes()); // section2 count
    out
}

/// A section-1 entry: a clip that carries NO root-motion/offset data — just its name
/// and animation path (no extension).
pub struct OffsetsClipNoMotion {
    pub clip_name: String,
    pub anim_path: String,
}

/// A section-2 entry: a clip WITH a root-motion block. The inner lane layout is fully
/// pinned (RE: `offsets_struct.py`, byte-identical re-emit on 3159/3162 corpus files):
/// `translations` = `(time, X, Y, Z)`, `rotations` = `(time, qx, qy, qz, qw)`,
/// `annotations` = `(time, event_name)`.
pub struct OffsetsMotion {
    pub anim_path: String,
    pub duration: f32,
    pub translations: Vec<(f32, [f32; 3])>,
    pub rotations: Vec<(f32, [f32; 4])>,
    pub annotations: Vec<(f32, String)>,
}

/// Build the **populated** `AnimationOffsets/<id>.txt` body (binary V4), byte-exact to
/// CK's grammar (`offsets_struct.py`): `"V4\n"` + `pstr(core)` + `u32(n1)` + section-1
/// `[pstr(clip) pstr(path)]` + `u32(n2)` + section-2 `[pstr(path) f32(dur) u32(nT)
/// {f32 time, f32 X, f32 Y, f32 Z} u32(nR) {f32 time, f32 qx, f32 qy, f32 qz, f32 qw}
/// u32(nAnn) {f32 time, pstr(name)}]`.
///
/// Section-2 samples are keyframe-reduced by `offsets::reduce_lanes` (selection/count/time
/// byte-exact vs CK; values within 1 ULP); `annotations` come from the annotation tracks
/// and `duration` is the clip duration.
pub fn animation_offsets_populated_body(
    core_behavior: &str,
    section1: &[OffsetsClipNoMotion],
    section2: &[OffsetsMotion],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + core_behavior.len() + section2.len() * 64);
    out.extend_from_slice(b"V4\n");
    push_pstr(&mut out, core_behavior);
    out.extend_from_slice(&(section1.len() as u32).to_le_bytes());
    for e in section1 {
        push_pstr(&mut out, &e.clip_name);
        push_pstr(&mut out, &e.anim_path);
    }
    out.extend_from_slice(&(section2.len() as u32).to_le_bytes());
    for e in section2 {
        push_pstr(&mut out, &e.anim_path);
        out.extend_from_slice(&e.duration.to_le_bytes());
        out.extend_from_slice(&(e.translations.len() as u32).to_le_bytes());
        for (t, xyz) in &e.translations {
            out.extend_from_slice(&t.to_le_bytes());
            for c in xyz {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&(e.rotations.len() as u32).to_le_bytes());
        for (t, q) in &e.rotations {
            out.extend_from_slice(&t.to_le_bytes());
            for c in q {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&(e.annotations.len() as u32).to_le_bytes());
        for (t, name) in &e.annotations {
            out.extend_from_slice(&t.to_le_bytes());
            push_pstr(&mut out, name);
        }
    }
    out
}

// ===========================================================================
// AnimationStanceData (binary)
// ===========================================================================

/// AnimationStanceData header string (one per file; len byte 0x17).
const STANCE_HEADER: &str = "AnimationBoneTransform";

pub type StanceTransform = ([f32; 4], [f32; 3]);

#[derive(Debug, Clone, PartialEq)]
pub struct StancePose {
    pub pose_idx: u8,
    pub variant: u8,
    pub slots: [StanceTransform; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub enum StanceSec2Payload {
    Trivial {
        padding: [u8; 18],
    },
    Grid {
        marker: u16,
        header: [f32; 4],
        cells: Vec<StanceTransform>,
    },
}

/// One structurally decoded section-2 record. Decoded donor records retain their
/// complete source bytes so SIDESTEP can copy a selected grid without re-encoding
/// any f32 lane.
#[derive(Debug, Clone, PartialEq)]
pub struct StanceSec2Record {
    pub tag: u32,
    pub reference: StanceTransform,
    pub payload: StanceSec2Payload,
    raw_record: Option<Box<[u8]>>,
}

impl StanceSec2Record {
    pub fn trivial(tag: u32, reference: StanceTransform) -> Self {
        Self {
            tag,
            reference,
            payload: StanceSec2Payload::Trivial { padding: [0; 18] },
            raw_record: None,
        }
    }

    pub fn grid(tag: u32, reference: StanceTransform, cells: Vec<StanceTransform>) -> Self {
        Self {
            tag,
            reference,
            payload: StanceSec2Payload::Grid {
                marker: STANCE_GRID_MARKER,
                header: STANCE_GRID_HEADER,
                cells,
            },
            raw_record: None,
        }
    }

    pub fn is_grid(&self) -> bool {
        matches!(self.payload, StanceSec2Payload::Grid { .. })
    }

    pub fn raw_record_bytes(&self) -> Option<&[u8]> {
        self.raw_record.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationStanceData {
    pub poses: Vec<StancePose>,
    pub sec2: Vec<StanceSec2Record>,
}

impl AnimationStanceData {
    pub fn sec2_by_tag(&self, tag: u32) -> Option<&StanceSec2Record> {
        self.sec2.iter().find(|record| record.tag == tag)
    }

    pub fn encode(&self) -> Result<Vec<u8>, StanceDataCodecError> {
        animation_stance_data_multipose_body(&self.poses, &self.sec2)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StanceDataCodecError {
    #[error("AnimationStanceData is truncated at byte {offset}")]
    Truncated { offset: usize },
    #[error("unexpected AnimationStanceData header {0:?}")]
    Header(String),
    #[error("unsupported AnimationStanceData version {0}")]
    Version(u32),
    #[error("invalid pose key 0x{0:08X}")]
    PoseKey(u32),
    #[error("duplicate pose key 0x{0:08X}")]
    DuplicatePose(u32),
    #[error("section-2 byte count {remaining} is not divisible by record count {count}")]
    Section2Stride { remaining: usize, count: usize },
    #[error("unsupported section-2 record size {0}")]
    Section2Size(usize),
    #[error("duplicate section-2 tag 0x{0:08X}")]
    DuplicateTag(u32),
    #[error("grid tag 0x{tag:08X} has marker 0x{marker:04X}, expected 0x0507")]
    GridMarker { tag: u32, marker: u16 },
    #[error("grid tag 0x{tag:08X} has non-canonical header")]
    GridHeader { tag: u32 },
    #[error("grid tag 0x{tag:08X} has {cells} cells, expected 35")]
    GridCellCount { tag: u32, cells: usize },
    #[error("copied section-2 tag 0x{tag:08X} has invalid retained byte length {len}")]
    CopiedRecordLength { tag: u32, len: usize },
    #[error("trailing bytes after AnimationStanceData body: {0}")]
    TrailingBytes(usize),
}

pub const STANCE_GRID_RECORD_LEN: usize = 1030;
pub const STANCE_TRIVIAL_RECORD_LEN: usize = 50;
pub const STANCE_GRID_MARKER: u16 = 0x0507;
pub const STANCE_GRID_HEADER: [f32; 4] = [-60.0, -60.0, 20.0, 30.0];

pub fn stance_pose_key(pose_idx: u8, variant: u8) -> u32 {
    u32::from(pose_idx) | (u32::from(variant) << 8)
}

pub fn stance_sec2_tag(pose_idx: u8, variant: u8, channel: u8) -> u32 {
    u32::from(pose_idx)
        | (u32::from(variant) << 8)
        | (u32::from(channel) << 16)
        | if channel == 2 { 2 << 24 } else { 0 }
}

fn read_u16(body: &[u8], offset: &mut usize) -> Result<u16, StanceDataCodecError> {
    let end = offset.saturating_add(2);
    let bytes = body
        .get(*offset..end)
        .ok_or(StanceDataCodecError::Truncated { offset: *offset })?;
    *offset = end;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32(body: &[u8], offset: &mut usize) -> Result<u32, StanceDataCodecError> {
    let end = offset.saturating_add(4);
    let bytes = body
        .get(*offset..end)
        .ok_or(StanceDataCodecError::Truncated { offset: *offset })?;
    *offset = end;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_f32(body: &[u8], offset: &mut usize) -> Result<f32, StanceDataCodecError> {
    Ok(f32::from_bits(read_u32(body, offset)?))
}

fn read_stance_transform(
    body: &[u8],
    offset: &mut usize,
) -> Result<StanceTransform, StanceDataCodecError> {
    let mut quat = [0.0; 4];
    let mut trans = [0.0; 3];
    for value in &mut quat {
        *value = read_f32(body, offset)?;
    }
    for value in &mut trans {
        *value = read_f32(body, offset)?;
    }
    Ok((quat, trans))
}

pub fn decode_animation_stance_data_body(
    body: &[u8],
) -> Result<AnimationStanceData, StanceDataCodecError> {
    let Some(&encoded_len) = body.first() else {
        return Err(StanceDataCodecError::Truncated { offset: 0 });
    };
    let encoded_len = usize::from(encoded_len);
    if encoded_len == 0 || body.len() < encoded_len + 1 {
        return Err(StanceDataCodecError::Truncated { offset: 1 });
    }
    let raw_name = &body[1..1 + encoded_len];
    if raw_name.last() != Some(&0) {
        return Err(StanceDataCodecError::Header(
            String::from_utf8_lossy(raw_name).into_owned(),
        ));
    }
    let name = String::from_utf8_lossy(&raw_name[..raw_name.len() - 1]).into_owned();
    if name != STANCE_HEADER {
        return Err(StanceDataCodecError::Header(name));
    }

    let mut offset = 1 + encoded_len;
    let version = read_u32(body, &mut offset)?;
    if version != 1 {
        return Err(StanceDataCodecError::Version(version));
    }
    let pose_count = read_u32(body, &mut offset)? as usize;
    let mut poses = Vec::with_capacity(pose_count);
    let mut pose_keys = HashSet::with_capacity(pose_count);
    for _ in 0..pose_count {
        let key = read_u32(body, &mut offset)?;
        if key & 0xffff_0000 != 0 {
            return Err(StanceDataCodecError::PoseKey(key));
        }
        if !pose_keys.insert(key) {
            return Err(StanceDataCodecError::DuplicatePose(key));
        }
        let slots = [
            read_stance_transform(body, &mut offset)?,
            read_stance_transform(body, &mut offset)?,
            read_stance_transform(body, &mut offset)?,
        ];
        poses.push(StancePose {
            pose_idx: key as u8,
            variant: (key >> 8) as u8,
            slots,
        });
    }

    let sec2_count = read_u32(body, &mut offset)? as usize;
    if sec2_count == 0 {
        if offset != body.len() {
            return Err(StanceDataCodecError::TrailingBytes(body.len() - offset));
        }
        return Ok(AnimationStanceData {
            poses,
            sec2: Vec::new(),
        });
    }
    let remaining = body.len().saturating_sub(offset);
    if remaining % sec2_count != 0 {
        return Err(StanceDataCodecError::Section2Stride {
            remaining,
            count: sec2_count,
        });
    }
    let record_len = remaining / sec2_count;
    if !matches!(
        record_len,
        STANCE_TRIVIAL_RECORD_LEN | STANCE_GRID_RECORD_LEN
    ) {
        return Err(StanceDataCodecError::Section2Size(record_len));
    }

    let mut sec2 = Vec::with_capacity(sec2_count);
    let mut tags = HashSet::with_capacity(sec2_count);
    for _ in 0..sec2_count {
        let record_start = offset;
        let tag = read_u32(body, &mut offset)?;
        if !tags.insert(tag) {
            return Err(StanceDataCodecError::DuplicateTag(tag));
        }
        let reference = read_stance_transform(body, &mut offset)?;
        let payload = if record_len == STANCE_TRIVIAL_RECORD_LEN {
            let mut padding = [0; 18];
            let end = offset + padding.len();
            padding.copy_from_slice(
                body.get(offset..end)
                    .ok_or(StanceDataCodecError::Truncated { offset })?,
            );
            offset = end;
            StanceSec2Payload::Trivial { padding }
        } else {
            let marker = read_u16(body, &mut offset)?;
            if marker != STANCE_GRID_MARKER {
                return Err(StanceDataCodecError::GridMarker { tag, marker });
            }
            let mut header = [0.0; 4];
            for value in &mut header {
                *value = read_f32(body, &mut offset)?;
            }
            if header.map(f32::to_bits) != STANCE_GRID_HEADER.map(f32::to_bits) {
                return Err(StanceDataCodecError::GridHeader { tag });
            }
            let mut cells = Vec::with_capacity(35);
            for _ in 0..35 {
                cells.push(read_stance_transform(body, &mut offset)?);
            }
            StanceSec2Payload::Grid {
                marker,
                header,
                cells,
            }
        };
        let record_end = record_start + record_len;
        if offset != record_end {
            return Err(StanceDataCodecError::Section2Size(offset - record_start));
        }
        sec2.push(StanceSec2Record {
            tag,
            reference,
            payload,
            raw_record: Some(body[record_start..record_end].into()),
        });
    }
    if offset != body.len() {
        return Err(StanceDataCodecError::TrailingBytes(body.len() - offset));
    }
    Ok(AnimationStanceData { poses, sec2 })
}

pub fn animation_stance_data_multipose_body(
    poses: &[StancePose],
    sec2: &[StanceSec2Record],
) -> Result<Vec<u8>, StanceDataCodecError> {
    let mut pose_keys = HashSet::with_capacity(poses.len());
    for pose in poses {
        let key = stance_pose_key(pose.pose_idx, pose.variant);
        if !pose_keys.insert(key) {
            return Err(StanceDataCodecError::DuplicatePose(key));
        }
    }
    let mut tags = HashSet::with_capacity(sec2.len());
    for record in sec2 {
        if !tags.insert(record.tag) {
            return Err(StanceDataCodecError::DuplicateTag(record.tag));
        }
        if let StanceSec2Payload::Grid { cells, .. } = &record.payload {
            if cells.len() != 35 {
                return Err(StanceDataCodecError::GridCellCount {
                    tag: record.tag,
                    cells: cells.len(),
                });
            }
        }
    }

    let retained_len: usize = sec2
        .iter()
        .map(|record| match record.payload {
            StanceSec2Payload::Trivial { .. } => STANCE_TRIVIAL_RECORD_LEN,
            StanceSec2Payload::Grid { .. } => STANCE_GRID_RECORD_LEN,
        })
        .sum();
    let mut out = Vec::with_capacity(36 + poses.len() * 88 + retained_len);
    push_pstr(&mut out, STANCE_HEADER);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(poses.len() as u32).to_le_bytes());
    for pose in poses {
        out.extend_from_slice(&stance_pose_key(pose.pose_idx, pose.variant).to_le_bytes());
        for slot in pose.slots {
            push_bone_transform(&mut out, slot.0, slot.1);
        }
    }
    out.extend_from_slice(&(sec2.len() as u32).to_le_bytes());
    for record in sec2 {
        if let Some(raw) = record.raw_record_bytes() {
            let expected = match record.payload {
                StanceSec2Payload::Trivial { .. } => STANCE_TRIVIAL_RECORD_LEN,
                StanceSec2Payload::Grid { .. } => STANCE_GRID_RECORD_LEN,
            };
            if raw.len() != expected {
                return Err(StanceDataCodecError::CopiedRecordLength {
                    tag: record.tag,
                    len: raw.len(),
                });
            }
            out.extend_from_slice(raw);
            continue;
        }
        out.extend_from_slice(&record.tag.to_le_bytes());
        push_bone_transform(&mut out, record.reference.0, record.reference.1);
        match &record.payload {
            StanceSec2Payload::Trivial { padding } => out.extend_from_slice(padding),
            StanceSec2Payload::Grid {
                marker,
                header,
                cells,
            } => {
                if *marker != STANCE_GRID_MARKER {
                    return Err(StanceDataCodecError::GridMarker {
                        tag: record.tag,
                        marker: *marker,
                    });
                }
                if header.map(f32::to_bits) != STANCE_GRID_HEADER.map(f32::to_bits) {
                    return Err(StanceDataCodecError::GridHeader { tag: record.tag });
                }
                out.extend_from_slice(&marker.to_le_bytes());
                for value in header {
                    out.extend_from_slice(&value.to_le_bytes());
                }
                for cell in cells {
                    push_bone_transform(&mut out, cell.0, cell.1);
                }
            }
        }
    }
    Ok(out)
}

/// Build the empty AnimationStanceData body, 36 bytes (RE: `binary_AnimationStanceData.md`,
/// `14636681807525876636.txt`): `pstr(header)`, `u32 version(1)`, `u32 entry-count(0)`, and
/// a trailing `u32(0)`.
///
/// Only the count>1 group container (group index + bone count + packed metadata), used
/// solely by the base-game human `Character`, is still partially decoded.
pub fn animation_stance_data_empty_body() -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    push_pstr(&mut out, STANCE_HEADER);
    out.extend_from_slice(&1u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // entry count
    out.extend_from_slice(&0u32.to_le_bytes()); // trailing section count
    out
}

/// Serialize one stance bone transform: quaternion (w,x,y,z), then translation (x,y,z),
/// 7 LE f32 = 28 bytes (RE: `stance_deep.md`; wxyz storage cross-validated exact on
/// MirelurkKing / LibertyPrime / SentryBot / MoleRat). There is no per-bone scale: the
/// `1.0` after the last bone is the `w` of the identity slot ([`STANCE_IDENTITY`]).
pub fn push_bone_transform(out: &mut Vec<u8>, quat_wxyz: [f32; 4], trans_xyz: [f32; 3]) {
    for c in quat_wxyz {
        out.extend_from_slice(&c.to_le_bytes());
    }
    for c in trans_xyz {
        out.extend_from_slice(&c.to_le_bytes());
    }
}

/// Identity stance slot: quat (1,0,0,0) wxyz + translation (0,0,0). Slot2 of a count=1
/// pose is always this; it is the `1.0` + zeros tail after slot1. (RE: `stance_deep.md`.)
const STANCE_IDENTITY: ([f32; 4], [f32; 3]) = ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);

/// Build the count=1 AnimationStanceData body (124 B, 2-slot pose), the form every
/// self-contained creature emits; count>1 occurs only on the base-game human `Character`
/// (RE: `stance_deep.md`).
///
/// Layout: `pstr("AnimationBoneTransform")`(24) + `u32 version=1` + `u32 count=1` +
/// `u32 0` (pose-0 header: poseIdx 0, variant 0) + slot0 + slot1 + slot2(IDENTITY) +
/// `u32 0` (section-2 count). slot0 is the Head model-space frame-0 pose; slot1 is the
/// torso/spine camera pivot (Head's COM-adjacent ancestor); each slot is 28 B
/// ([`push_bone_transform`], quat W-first). The container is byte-identical to the CK
/// oracle (MirelurkKing, 124 B); bone floats match to the `hkaPose` accumulation residual
/// (~1e-3 on deep bones).
pub fn animation_stance_data_count1_body(
    slot0: ([f32; 4], [f32; 3]),
    slot1: ([f32; 4], [f32; 3]),
) -> Vec<u8> {
    let mut out = Vec::with_capacity(124);
    push_pstr(&mut out, STANCE_HEADER); // 24 B: len byte 0x17 + "AnimationBoneTransform\0"
    out.extend_from_slice(&1u32.to_le_bytes()); // version
    out.extend_from_slice(&1u32.to_le_bytes()); // count = 1 (creatures always)
    out.extend_from_slice(&0u32.to_le_bytes()); // pose 0 header (poseIdx 0 | variant 0)
    push_bone_transform(&mut out, slot0.0, slot0.1); // slot0 = Head
    push_bone_transform(&mut out, slot1.0, slot1.1); // slot1 = torso/spine pivot
    push_bone_transform(&mut out, STANCE_IDENTITY.0, STANCE_IDENTITY.1); // slot2 = IDENTITY
    out.extend_from_slice(&0u32.to_le_bytes()); // section-2 count = 0
    out
}

/// The CK section-2 record tag for a head-track / look-at reference: bytes `00 00 00 02`
/// (LE `u32` `0x0200_0000`) — constant across every observed entry.
const STANCE_SEC2_TAG: u32 = 0x0200_0000;

/// Build the head-tracking AnimationStanceData body (174 B), the FO76→FO4 converted-creature
/// form, emitted when the core behavior declares head-tracking (`bGraphWantsHeadTracking` /
/// `isActiveModifier_HeadTracking`). It is the 124 B [`animation_stance_data_count1_body`]
/// container with `sec2_count = 1` plus one 50-byte section-2 record: `u32 tag(0x0200_0000)`,
/// the head-track look-at reference bone (28 B, quat W-first), and 18 zero pad bytes.
///
/// RE: `stance_converted_174b.md`. The container is byte-exact on the Snallygaster CK oracle;
/// slot0/slot1 are float-exact from idle frame 0; only the `sec2` look-at residual is
/// approximated (the engine recomputes head-track at runtime). Vanilla creatures without
/// head-tracking keep the 124 B form.
pub fn animation_stance_data_headtrack_body(
    slot0: ([f32; 4], [f32; 3]),
    slot1: ([f32; 4], [f32; 3]),
    sec2: ([f32; 4], [f32; 3]),
) -> Vec<u8> {
    let mut out = Vec::with_capacity(174);
    push_pstr(&mut out, STANCE_HEADER); // 24 B: len byte 0x17 + "AnimationBoneTransform\0"
    out.extend_from_slice(&1u32.to_le_bytes()); // version
    out.extend_from_slice(&1u32.to_le_bytes()); // count = 1
    out.extend_from_slice(&0u32.to_le_bytes()); // pose 0 header (poseIdx 0 | variant 0)
    push_bone_transform(&mut out, slot0.0, slot0.1); // slot0 = Head
    push_bone_transform(&mut out, slot1.0, slot1.1); // slot1 = spine pivot (parent of first neck)
    push_bone_transform(&mut out, STANCE_IDENTITY.0, STANCE_IDENTITY.1); // slot2 = IDENTITY
    out.extend_from_slice(&1u32.to_le_bytes()); // section-2 count = 1
    out.extend_from_slice(&STANCE_SEC2_TAG.to_le_bytes()); // tag bytes 00 00 00 02
    push_bone_transform(&mut out, sec2.0, sec2.1); // head-track look-at reference
    out.extend_from_slice(&[0u8; 18]); // zero pad
    out
}

/// Build the `DynamicIdleData/<id>.txt` body bytes.
/// Format (matches CK): `"Version 1\n"` then each idle path (no extension) + `'\n'`.
pub fn dynamic_idle_data_body(idle_paths: &[String]) -> Vec<u8> {
    let mut s = String::with_capacity(16 + idle_paths.len() * 48);
    s.push_str("Version 1\n");
    for p in idle_paths {
        s.push_str(p);
        s.push('\n');
    }
    s.into_bytes()
}

/// Build the `AnimEventInfo/<id>.txt` body bytes (text V2).
/// Format (matches CK): `"V2\n" "<behavior_path>\n" "\n" "<count>\n"` then per event
/// `"<name>\n" "<flag>\n" "<nClips>\n"` followed by each clip name + `'\n'`.
pub fn anim_event_info_body(behavior_path: &str, events: &[AnimEvent]) -> Vec<u8> {
    let mut s = String::with_capacity(32 + events.len() * 32);
    s.push_str("V2\n");
    s.push_str(behavior_path);
    s.push_str("\n\n"); // path newline + one blank line
    s.push_str(&events.len().to_string());
    s.push('\n');
    for ev in events {
        s.push_str(&ev.name);
        s.push('\n');
        s.push_str(&ev.flag.to_string());
        s.push('\n');
        s.push_str(&ev.clips.len().to_string());
        s.push('\n');
        for c in &ev.clips {
            s.push_str(c);
            s.push('\n');
        }
    }
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Parse a CK/our AnimationFileData body into (version, flag, id, files).
    fn parse(body: &[u8]) -> (String, String, u64, Vec<String>) {
        let text = String::from_utf8(body.to_vec()).unwrap();
        let mut lines = text.split('\n');
        let version = lines.next().unwrap().to_string();
        let flag = lines.next().unwrap().to_string();
        let id: u64 = lines.next().unwrap().parse().unwrap();
        let count: usize = lines.next().unwrap().parse().unwrap();
        let files: Vec<String> = lines.by_ref().take(count).map(str::to_string).collect();
        (version, flag, id, files)
    }

    #[test]
    fn body_format_is_byte_exact_for_known_input() {
        let files = vec![
            r"Actors\X\Animations\Aaa.hkx".to_string(),
            r"Actors\X\Animations\Bbb.hkx".to_string(),
        ];
        let body = animation_file_data_body(42, &files);
        let expected =
            "3\n1\n42\n2\nActors\\X\\Animations\\Aaa.hkx\nActors\\X\\Animations\\Bbb.hkx\n";
        assert_eq!(body, expected.as_bytes());
        // Trailing newline after the last file is present.
        assert_eq!(*body.last().unwrap(), b'\n');
    }

    #[test]
    fn roundtrips_through_parse() {
        let files = vec![r"Actors\X\Animations\Aaa.hkx".to_string()];
        let (v, fl, id, got) = parse(&animation_file_data_body(7, &files));
        assert_eq!((v.as_str(), fl.as_str(), id), ("3", "1", 7));
        assert_eq!(got, files);
    }

    #[test]
    fn body_drops_case_and_separator_duplicates_keeping_first_spelling() {
        let files = vec![
            r"Actors\X\Animations\ChargeStrike.hkx".to_string(),
            r"Actors\X\Animations\TuskSwipe_Front.hkx".to_string(),
            r"Actors\X\Animations\chargestrike.hkx".to_string(),
            r"Actors/X/Animations/tuskswipe_front.hkx".to_string(),
        ];
        let (_, _, _, got) = parse(&animation_file_data_body(1, &files));
        assert_eq!(got, files[..2].to_vec());
        // The declared count must describe the rows actually written.
        let text = String::from_utf8(animation_file_data_body(1, &files)).unwrap();
        assert_eq!(text.lines().nth(3).unwrap(), "2");
    }

    // ---- SyncAnimData -----------------------------------------------------

    #[test]
    fn sync_anim_data_is_v4_zero() {
        assert_eq!(sync_anim_data_body(), b"V4\n0\n");
    }

    // ---- Named project manifest ------------------------------------------

    #[test]
    fn project_manifest_format_is_crlf_with_trailing_zero() {
        let files = vec![
            r"Behaviors\X.hkx".to_string(),
            r"Animations\Idle.hkx".to_string(),
        ];
        let body = project_manifest_body("XProject", &files);
        let expected =
            "3\r\n0\r\nXProject\r\n2\r\nBehaviors\\X.hkx\r\nAnimations\\Idle.hkx\r\n0\r\n";
        assert_eq!(body, expected.as_bytes());
    }

    // ---- ClipGeneratorData (binary) --------------------------------------

    // ---- AnimationOffsets (binary) ---------------------------------------

    // ---- AnimationStanceData (binary) ------------------------------------

    #[test]
    fn animation_stance_data_empty_reproduces_extracted_oracle() {
        // Vanilla empty stance file (36 bytes): header + version 1 + count 0 + 0.
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../../extracted/fo4/Meshes/AnimTextData/animationstancedata/\
             14636681807525876636.txt",
        );
        let Ok(oracle) = std::fs::read(p) else {
            eprintln!("oracle absent; skipping");
            return;
        };
        assert_eq!(oracle.len(), 36);
        assert_eq!(animation_stance_data_empty_body(), oracle);
    }

    #[test]
    fn bone_transform_is_28_bytes_quat_wxyz_first() {
        let mut out = Vec::new();
        // Caller passes [qw, qx, qy, qz] then [tx, ty, tz] (wxyz quaternion order).
        push_bone_transform(&mut out, [1.0, 0.0, 0.0, 0.0], [1.0, 2.0, 3.0]);
        assert_eq!(out.len(), 28);
        // First 16 bytes = quaternion (w,x,y,z); last 12 = translation (x,y,z).
        assert_eq!(&out[0..4], &1.0f32.to_le_bytes()); // qw at index 0
        assert_eq!(&out[16..20], &1.0f32.to_le_bytes()); // tx
        assert_eq!(&out[24..28], &3.0f32.to_le_bytes()); // tz
    }

    /// The 174 B head-tracking container = the 124 B count=1 body's first 120 bytes
    /// (header..slot2), then `sec2_count=1` + tag `00 00 00 02` + the sec2 bone (28 B) +
    /// 18 zero pad. `174 = 124 + 50`. The framing constants match the Snallygaster oracle.
    #[test]
    fn animation_stance_data_headtrack_is_174b_with_tag_and_pad() {
        let slot0 = ([0.1f32, 0.2, 0.3, 0.4], [1.0f32, 2.0, 3.0]);
        let slot1 = ([0.5f32, 0.6, 0.7, 0.8], [4.0f32, 5.0, 6.0]);
        let sec2 = ([0.9f32, 0.1, 0.2, 0.3], [7.0f32, 8.0, 9.0]);
        let body = animation_stance_data_headtrack_body(slot0, slot1, sec2);
        assert_eq!(body.len(), 174, "174 = 124 + 50");

        // First 120 bytes identical to the 124 B body (header..slot2 IDENTITY).
        let body124 = animation_stance_data_count1_body(slot0, slot1);
        assert_eq!(
            &body[0..120],
            &body124[0..120],
            "shared 124 B prefix through slot2"
        );
        assert_eq!(
            &body124[120..124],
            &0u32.to_le_bytes(),
            "124 B form: sec2_count 0"
        );

        // Section-2: count=1, tag bytes 00 00 00 02, sec2 bone, 18-byte zero pad.
        assert_eq!(&body[120..124], &1u32.to_le_bytes(), "sec2_count 1");
        assert_eq!(&body[124..128], &[0u8, 0, 0, 2], "tag bytes");
        assert_eq!(&body[128..132], &0.9f32.to_le_bytes(), "sec2 qw");
        assert_eq!(&body[144..148], &7.0f32.to_le_bytes(), "sec2 tx");
        assert_eq!(&body[156..174], &[0u8; 18], "18-byte zero pad");
    }

    fn synthetic_pose(pose_idx: u8, variant: u8) -> StancePose {
        StancePose {
            pose_idx,
            variant,
            slots: [
                ([1.0, 0.0, 0.0, 0.0], [1.0, 2.0, 3.0]),
                ([0.0, 1.0, 0.0, 0.0], [4.0, 5.0, 6.0]),
                STANCE_IDENTITY,
            ],
        }
    }

    #[test]
    fn multipose_codec_roundtrips_trivial_and_grid_records() {
        let poses = vec![synthetic_pose(0, 0), synthetic_pose(0, 1)];
        let center = ([1.0, 0.0, 0.0, 0.0], [7.0, 8.0, 9.0]);
        let records = vec![
            StanceSec2Record::trivial(stance_sec2_tag(0, 0, 0), center),
            StanceSec2Record::grid(stance_sec2_tag(0, 1, 0), center, vec![center; 35]),
        ];

        // A real file never mixes record widths. Exercise each structural form
        // independently because the count determines one uniform record stride.
        for record in records {
            let encoded = animation_stance_data_multipose_body(&poses, &[record]).unwrap();
            let decoded = decode_animation_stance_data_body(&encoded).unwrap();
            assert_eq!(decoded.encode().unwrap(), encoded);
        }
    }

    #[test]
    fn multipose_codec_rejects_duplicate_keys_and_malformed_stride() {
        let pose = synthetic_pose(0, 0);
        assert_eq!(
            animation_stance_data_multipose_body(&[pose.clone(), pose], &[]),
            Err(StanceDataCodecError::DuplicatePose(0))
        );

        let center = STANCE_IDENTITY;
        let mut encoded = animation_stance_data_multipose_body(
            &[synthetic_pose(0, 0)],
            &[StanceSec2Record::trivial(0, center)],
        )
        .unwrap();
        encoded.pop();
        assert!(matches!(
            decode_animation_stance_data_body(&encoded),
            Err(StanceDataCodecError::Section2Size(_))
                | Err(StanceDataCodecError::Section2Stride { .. })
        ));
    }
}
