//! AnimationOffsets per-subgraph populated emission: root-motion fixup (CK-free).
//!
//! The engine treats a present-but-empty `AnimationOffsets/<id>.txt` as authoritative ("no
//! root motion"), which kills locomotion and turn displacement for converted creatures
//! (sliding / moonwalking). A moving subgraph must ship non-empty translation/rotation blocks.

//!
//! ## Source (RE: `bucket5_offsets_findings.md`, `reframe.py`)
//! Root motion is the clip's baked `hkaDefaultAnimatedReferenceFrame`, not the root bone
//! channel (`root_motion.rs::extract_from_root_channel` reads the wrong source). Each
//! `referenceFrameSamples[i]` is a `Vector4 (x,y,z,w)`: `(x,y,z)` is the per-frame world
//! displacement and `w` = `paraDistance`, the accumulated heading in radians. The rotation
//! key is the axis-angle quaternion about the reference-frame `up`:
//! `q = (up.x·sin(θ/2), up.y·sin(θ/2), up.z·sin(θ/2), cos(θ/2))`, `θ = w`. Values come from
//! the binary `HkxValue` (true f32); `unpack_hkx_to_xml`'s `%f` text truncates ~1 ULP.
//! Signed zeros fall out of the plain f32 multiply (right turns: `0.0·(−s) = −0.0`).
//!
//! ## Keyframe reduction
//! `reduce_lanes` reduces the dense per-frame samples to a sparse keyset via `grow_while_fits`
//! (selection/count/time byte-exact vs CK; values within 1 ULP). The engine interpolates
//! linearly between kept keys.
//!
//! ## Clip set (creature builder)
//! * `section1` (`clip_name` → `anim_path`, no motion block): the core behavior's named clip
//!   generators whose generator name equals their animation basename. That invariant holds
//!   across all 511064 vanilla section-1 entries and the runtime depends on it (see the guard
//!   in `build_subgraph_offsets_body`). Generators with an empty `animationName`, dynamic
//!   clips playing the `Idle` placeholder (RadHog's `DodgeLeft`/`EvadeLeft`), and aliased
//!   clips the engine could not resolve are dropped.
//! * `section2` (`anim_path` → root-motion block): the section-1 clips with a baked reference
//!   frame, plus the engine's directional turn-in-place clips (`TurnLeft<deg>`/`TurnRight<deg>`
//!   on disk; plain `TurnLeft`/`TurnRight` excluded). The turn-to-face system drives those, not
//!   the behavior graph, so they are collected by on-disk name across the SAPT chain.
//!
//! The weapon and furniture builders differ; see [`build_subgraph_offsets_body_weapon`] and
//! [`build_subgraph_offsets_body_furniture`].

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use havok_native::hkx::HkxObject;
use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;

use super::behavior_index::{clip_leaf, core_project_dir, resolve_leaf};
use super::bucket_files::{OffsetsClipNoMotion, OffsetsMotion, animation_offsets_populated_body};
use super::emit::SubgraphInput;
use super::graph::GraphResolver;
use super::hkx_cache::{FileMemo, path_key};

/// Subgraphs whose IDs appear in the vanilla aggregate but must be excluded
/// when rebuilding. These are WeaponBehavior/GunBehavior entries whose SAPT
/// chains contain "Injured" -- they survived the vanilla CK build but are
/// dropped when CK rebuilds with a mod loaded.
const VANILLA_INJURED_EXCLUSIONS: &[u64] = &[
    2798016766296788894,  // WeaponBehavior sapt_crc=651464044
    2815779307924033438,  // WeaponBehavior sapt_crc=655599708
    2850019573186760606,  // WeaponBehavior sapt_crc=663571891
    3634740580962310295,  // GunBehavior    sapt_crc=846278988
    5737393547891773342,  // WeaponBehavior sapt_crc=1335841032
    7308697184125652894,  // WeaponBehavior sapt_crc=1701688669
    9004150397693398942,  // WeaponBehavior sapt_crc=2096442132
    9331636681854720151,  // GunBehavior    sapt_crc=2172690974
    12717401627996524446, // WeaponBehavior sapt_crc=2961000806
];

/// Whether a subgraph belongs in the project-level AnimationOffsets aggregate.
pub fn is_aggregate_candidate(sg: &SubgraphInput) -> bool {
    if sg.core_behavior.to_ascii_lowercase().contains("injured") {
        return false;
    }
    !sg.sapt_chain
        .iter()
        .any(|path| path.to_ascii_lowercase().contains("injured"))
}

/// Build the sorted lines for `PersistantSubgraphInfoAndOffsetData.txt`.
///
/// Returns `None` unless the base aggregate is present, readable, non-empty, and valid.
pub fn build_offsets_aggregate(
    subgraphs: &[SubgraphInput],
    base_agg_path: Option<&Path>,
) -> Option<Vec<String>> {
    let mut ids = BTreeSet::new();

    let content = std::fs::read_to_string(base_agg_path?).ok()?;
    let mut base_entry_count = 0;
    for line in content.lines() {
        let entry = Path::new(line.trim());
        if entry.file_name() != Some(entry.as_os_str())
            || entry.extension().and_then(|ext| ext.to_str()) != Some("txt")
        {
            return None;
        }
        let id = entry
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u64>().ok())?;
        base_entry_count += 1;
        if !VANILLA_INJURED_EXCLUSIONS.contains(&id) {
            ids.insert(id);
        }
    }
    if base_entry_count == 0 {
        return None;
    }

    ids.extend(
        subgraphs
            .iter()
            .filter(|sg| is_aggregate_candidate(sg))
            .map(SubgraphInput::id),
    );

    Some(ids.into_iter().map(|id| format!("{id}.txt\n")).collect())
}

/// A clip's baked root-motion reference frame (`hkaDefaultAnimatedReferenceFrame`).
pub(super) struct BakedReferenceFrame {
    /// Rotation axis (typically `(0,0,1)`); `x`/`y` carry the signed-zero for turn quats.
    up: [f32; 3],
    pub(super) duration: f32,
    /// Dense per-frame `(x, y, z, headingAngleRadians)`.
    pub(super) samples: Vec<[f32; 4]>,
    reduced_lanes: OnceLock<ReducedLanes>,
}

type ReducedLanes = (Vec<(f32, [f32; 3])>, Vec<(f32, [f32; 4])>);

impl BakedReferenceFrame {
    fn reduced_lanes(&self) -> ReducedLanes {
        self.reduced_lanes
            .get_or_init(|| reduce_lanes(self))
            .clone()
    }
}

fn f32_member(obj: &HkxObject, name: &str) -> Option<f32> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::F32(f) => Some(*f),
            HkxValue::Half(f) => Some(*f),
            _ => None,
        })
}

fn f32list_member<'a>(obj: &'a HkxObject, name: &str) -> Option<&'a [f32]> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::F32List(v) => Some(v.as_slice()),
            _ => None,
        })
}

/// One clip's cached derivations, shared by every subgraph that reaches the clip. The
/// reference frame, annotations and duration come from a single parse.
struct ClipMemoEntry {
    reference_frame: Option<Arc<BakedReferenceFrame>>,
    annotations: Arc<Vec<(f32, String)>>,
    duration: Option<f32>,
}

static CLIPS: FileMemo<Arc<ClipMemoEntry>> = FileMemo::new();

fn clip_memo(clip_hkx: &Path) -> Arc<ClipMemoEntry> {
    CLIPS.get_or_init(&path_key(clip_hkx), || {
        let hkx = std::fs::read(clip_hkx)
            .ok()
            .and_then(|data| read_packfile(&data).ok());
        Arc::new(ClipMemoEntry {
            reference_frame: hkx
                .as_ref()
                .and_then(|hkx| baked_reference_frame(hkx.objects()))
                .map(Arc::new),
            annotations: Arc::new(
                hkx.as_ref()
                    .map(|hkx| root_annotations(hkx.objects()))
                    .unwrap_or_default(),
            ),
            duration: hkx
                .as_ref()
                .and_then(|hkx| animation_duration(hkx.objects())),
        })
    })
}

pub(super) fn clear_clip_memo() {
    CLIPS.clear();
}

/// A stationary clip carries no `hkaDefaultAnimatedReferenceFrame`, so its length has to come
/// off the animation itself. CK still writes a section-2 block for those clips with their real
/// duration, e.g. `WPNPitchDownReadyAdd` at 0.333s.
fn animation_duration(objects: &[HkxObject]) -> Option<f32> {
    objects
        .iter()
        .find(|o| o.class_name.starts_with("hka") && o.class_name.ends_with("Animation"))
        .and_then(|o| f32_member(o, "duration"))
}

pub(super) fn extract_clip_duration(clip_hkx: &Path) -> Option<f32> {
    clip_memo(clip_hkx).duration
}

/// Read the baked reference frame from a clip `.hkx` (binary model, true f32), memoized.
pub(super) fn extract_baked_reference_frame(clip_hkx: &Path) -> Option<Arc<BakedReferenceFrame>> {
    clip_memo(clip_hkx).reference_frame.clone()
}

fn baked_reference_frame(objects: &[HkxObject]) -> Option<BakedReferenceFrame> {
    let obj = objects
        .iter()
        .find(|o| o.class_name == "hkaDefaultAnimatedReferenceFrame")?;

    let up = f32list_member(obj, "up").filter(|u| u.len() >= 3)?;
    let up = [up[0], up[1], up[2]];
    let duration = f32_member(obj, "duration")?;

    let samples_member = obj
        .members
        .iter()
        .find(|m| m.name == "referenceFrameSamples")?;
    let HkxValue::Array(items) = &samples_member.value else {
        return None;
    };
    let samples: Vec<[f32; 4]> = items
        .iter()
        .filter_map(|it| match it {
            HkxValue::F32List(v) if v.len() >= 4 => Some([v[0], v[1], v[2], v[3]]),
            _ => None,
        })
        .collect();
    if samples.is_empty() {
        return None;
    }
    Some(BakedReferenceFrame {
        up,
        duration,
        samples,
        reduced_lanes: OnceLock::new(),
    })
}

/// Streaming **grow-while-fits** piecewise-linear decimation. Returns the kept frame
/// indices: frame 0 is the IMPLICIT origin (NEVER emitted) and the terminal frame `N-1` is
/// always retained. `err(a,b,j)` = the error of dropping interior frame `j` from the chord
/// `a→b`; a segment `[a,b]` fits iff every interior `j` has `err ≤ tol`. Not RDP and not
/// furthest-reachable (RE: `offsets_byte_exact.md`).
fn grow_while_fits(n: usize, tol: f32, err: impl Fn(usize, usize, usize) -> f32) -> Vec<usize> {
    let mut keys: Vec<usize> = Vec::new();
    if n <= 1 {
        return keys;
    }
    let mut a = 0usize;
    let mut b = 1usize;
    while b < n {
        if (a + 1..b).all(|j| err(a, b, j) <= tol) {
            b += 1; // segment still fits — extend
        } else {
            keys.push(b - 1); // b broke it — lock the last frame that fit
            a = b - 1;
            b = a + 1;
        }
    }
    if keys.last() != Some(&(n - 1)) {
        keys.push(n - 1); // terminal frame is always retained
    }
    keys
}

/// CK keyframe-reduced translation `(time,X,Y,Z)` + rotation `(time,qx,qy,qz,qw)` lanes.
/// Selection, count and time are byte-exact vs CK (`offsets_byte_exact.md`):
/// * translation: grow-while-fits, Euclidean-3D chord error, `tol = 2.0`.
/// * rotation: grow-while-fits, heading-angle error `|Δw|`, `tol = 0.3°`, plus the antipode
///   rule: a segment spanning ≥180° (`dot(q_a,q_b) ≤ 0`) forces a key at the last frame
///   before `qw` goes negative. Gated on the f32 `qw` sign, so `+π` TurnLeft180 forces a key
///   but `−π` TurnRight180 does not.
/// * time: `t[fr] = f32(fr · f32(duration/(N-1)))`, not `fr/fps` (1 ULP off).
/// * values: sampled at the key time via Havok `getReferenceFrame` interpolation, not read
///   verbatim from `samples[fr]`; within 1 ULP of CK. Selection runs on the dense samples.
fn reduce_lanes(rf: &BakedReferenceFrame) -> ReducedLanes {
    let n = rf.samples.len();
    if n == 0 {
        return (Vec::new(), Vec::new());
    }
    let dt = if n > 1 {
        rf.duration / (n as f32 - 1.0) // f32 frameDuration
    } else {
        0.0
    };
    let time_of = |fr: usize| -> f32 { fr as f32 * dt };
    let pos = |fr: usize| -> [f32; 3] {
        let s = &rf.samples[fr];
        [s[0], s[1], s[2]]
    };
    let quat = |fr: usize| -> [f32; 4] {
        let half = rf.samples[fr][3] * 0.5;
        // Plain f32 multiply reproduces the oracle's signed zeros (up.x/up.y == 0.0).
        let sin = half.sin();
        [rf.up[0] * sin, rf.up[1] * sin, rf.up[2] * sin, half.cos()]
    };

    // Translation: Euclidean-3D chord error, tol 2.0.
    let trans_keys = grow_while_fits(n, 2.0, |a, b, j| {
        let t = (j - a) as f32 / (b - a) as f32;
        let (pa, pb, pj) = (pos(a), pos(b), pos(j));
        let dx = pj[0] - (pa[0] + (pb[0] - pa[0]) * t);
        let dy = pj[1] - (pa[1] + (pb[1] - pa[1]) * t);
        let dz = pj[2] - (pa[2] + (pb[2] - pa[2]) * t);
        (dx * dx + dy * dy + dz * dz).sqrt()
    });

    // Rotation: heading-angle |Δw| error, tol 0.3°.
    let w = |fr: usize| -> f32 { rf.samples[fr][3] };
    let mut rot_keys = grow_while_fits(n, 0.3_f32.to_radians(), |a, b, j| {
        let t = (j - a) as f32 / (b - a) as f32;
        (w(j) - (w(a) + (w(b) - w(a)) * t)).abs()
    });
    // Antipode disambiguation: for each kept segment [prev,k] with dot(q_prev,q_k) ≤ 0,
    // force a key at the last frame before qw goes negative (an EXTRA key — does not
    // re-anchor the grow-while-fits segments).
    let mut extra: Vec<usize> = Vec::new();
    let mut prev = 0usize;
    for &k in &rot_keys {
        let (qa, qb) = (quat(prev), quat(k));
        let dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
        if dot <= 0.0 {
            if let Some(f) = (prev + 1..=k).find(|&f| quat(f)[3] < 0.0) {
                if f - 1 > prev {
                    extra.push(f - 1);
                }
            }
        }
        prev = k;
    }
    if !extra.is_empty() {
        rot_keys.extend(extra);
        rot_keys.sort_unstable();
        rot_keys.dedup();
    }

    // Values come from Havok `getReferenceFrame(t)` at the key time, not the verbatim
    // `samples[fr]`. The f32 `time→p` round-trip lands `p` a hair off the integer, so the lerp
    // nudges an on-frame key by up to 1 ULP, which is what CK stores.
    let sample_at = |t: f32| -> [f32; 4] {
        if n == 1 || rf.duration == 0.0 {
            return rf.samples[0];
        }
        let p = (t / rf.duration) * (n as f32 - 1.0);
        let i = (p.floor() as isize).clamp(0, n as isize - 2) as usize;
        let frac = p - i as f32;
        let (a, b) = (&rf.samples[i], &rf.samples[i + 1]);
        [
            a[0] + (b[0] - a[0]) * frac,
            a[1] + (b[1] - a[1]) * frac,
            a[2] + (b[2] - a[2]) * frac,
            a[3] + (b[3] - a[3]) * frac,
        ]
    };
    let value_pos = |fr: usize| -> [f32; 3] {
        let s = sample_at(time_of(fr));
        [s[0], s[1], s[2]]
    };
    let value_quat = |fr: usize| -> [f32; 4] {
        // Interpolate the heading `w` at the key time, then q = (up·sin(w/2), cos(w/2)).
        let half = sample_at(time_of(fr))[3] * 0.5;
        let sin = half.sin();
        [rf.up[0] * sin, rf.up[1] * sin, rf.up[2] * sin, half.cos()]
    };

    let trans = trans_keys
        .iter()
        .map(|&fr| (time_of(fr), value_pos(fr)))
        .collect();
    let rot = rot_keys
        .iter()
        .map(|&fr| (time_of(fr), value_quat(fr)))
        .collect();
    (trans, rot)
}

/// True iff the clip actually moves: any frame past 0 has nonzero displacement or a
/// nonzero heading angle. (Identity-frame-0-only clips → no motion.)
fn frame_has_motion(rf: &BakedReferenceFrame) -> bool {
    rf.samples
        .iter()
        .any(|s| s[0] != 0.0 || s[1] != 0.0 || s[2] != 0.0 || s[3] != 0.0)
}

/// `(time, name)` annotations from the clip's root annotation track (`annotationTracks[0]`).
/// Best-effort (dense emission is not byte-identical, so CK's name canonicalization /
/// same-timestamp reorder is not replicated).
fn extract_root_annotations(clip_hkx: &Path) -> Vec<(f32, String)> {
    (*clip_memo(clip_hkx).annotations).clone()
}

fn root_annotations(objects: &[HkxObject]) -> Vec<(f32, String)> {
    for obj in objects {
        let Some(tracks_member) = obj.members.iter().find(|m| m.name == "annotationTracks") else {
            continue;
        };
        let HkxValue::Array(tracks) = &tracks_member.value else {
            continue;
        };
        let Some(first) = tracks.first() else {
            continue;
        };
        let Some(track_members) = first.as_object_members() else {
            continue;
        };
        let Some(ann_member) = track_members.iter().find(|m| m.name == "annotations") else {
            continue;
        };
        let HkxValue::Array(anns) = &ann_member.value else {
            continue;
        };
        let mut out = Vec::with_capacity(anns.len());
        for a in anns {
            let Some(ms) = a.as_object_members() else {
                continue;
            };
            let time = ms
                .iter()
                .find(|m| m.name == "time")
                .and_then(|m| match &m.value {
                    HkxValue::F32(f) => Some(*f),
                    HkxValue::Half(f) => Some(*f),
                    _ => None,
                });
            let text = ms
                .iter()
                .find(|m| m.name == "text")
                .and_then(|m| match &m.value {
                    HkxValue::String { value, .. } if !value.is_empty() => Some(value.clone()),
                    _ => None,
                });
            if let (Some(t), Some(n)) = (time, text) {
                out.push((t, n));
            }
        }
        return out;
    }
    Vec::new()
}

/// Walk a behavior `.hkx` for its `hkbClipGenerator` `(name, animationName)` pairs.
fn collect_clip_generators(behavior: &Path) -> Vec<(String, String)> {
    let Some(hkx) = super::hkx_cache::behavior_packfile(behavior) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for obj in hkx.objects() {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        let name = obj
            .members
            .iter()
            .find(|m| m.name == "name")
            .and_then(|m| match &m.value {
                HkxValue::String { value, .. } if !value.is_empty() => Some(value.clone()),
                _ => None,
            });
        let anim = obj
            .members
            .iter()
            .find(|m| m.name == "animationName")
            .and_then(|m| match &m.value {
                HkxValue::String { value, .. } if !value.is_empty() => Some(value.clone()),
                _ => None,
            });
        if let (Some(name), Some(anim)) = (name, anim) {
            out.push((name, anim));
        }
    }
    out
}

/// True for the engine's directional turn-in-place clips (`TurnLeft30`, `TurnRight180`,
/// …): basename = `turn(left|right)` + a non-empty digit run. Plain `TurnLeft`/
/// `TurnRight` (no degree suffix) are excluded, matching CK.
fn is_directional_turn(basename: &str) -> bool {
    let lc = basename.to_ascii_lowercase();
    ["turnleft", "turnright"].iter().any(|prefix| {
        lc.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Collect the directional turn clips on disk across the SAPT chain (first dir wins,
/// like the override search). Returns `(anim_path_no_ext_rel, disk_path)`.
fn directional_turn_anims(meshes_root: &Path, sapt_chain: &[String]) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for sapt in sapt_chain {
        let dir_rel = sapt.trim_end_matches(['\r', '\n', ' ']);
        let disk_dir = meshes_root.join(dir_rel.replace('\\', "/"));
        let Ok(entries) = std::fs::read_dir(&disk_dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let is_hkx = p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("hkx"));
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_hkx || !is_directional_turn(stem) {
                continue;
            }
            if seen.insert(stem.to_ascii_lowercase()) {
                out.push((format!("{dir_rel}\\{stem}"), p));
            }
        }
    }
    out
}

/// Extract one clip's dense motion + annotations and push a section2 entry. Combat and
/// turn clips are included unconditionally (CK keeps a static-but-annotated attack like
/// `Attack7`). Sets `any_motion` when the clip actually moves. No-op if the clip carries
/// no baked reference frame (every converted creature anim does — proven).
fn push_motion_entry(
    disk: &Path,
    anim_path_no_ext: String,
    section2: &mut Vec<OffsetsMotion>,
    any_motion: &mut bool,
) {
    let Some(rf) = extract_baked_reference_frame(disk) else {
        return;
    };
    *any_motion |= frame_has_motion(&rf);
    let annotations = extract_root_annotations(disk);
    let (translations, rotations) = rf.reduced_lanes();
    section2.push(OffsetsMotion {
        anim_path: anim_path_no_ext,
        duration: rf.duration,
        translations,
        rotations,
        annotations,
    });
}

/// Build the populated per-subgraph `AnimationOffsets/<id>.txt` body for a creature, or
/// `None` if no clip moves (the engine then rebuilds offsets from the empty project-level
/// entry, which is correct for a static subgraph).
///
/// `core_behavior_disk` is the clip source; `core_behavior_rel` is the FO4 relpath written as
/// the V4 `core_behavior` string. `event_clip_names` is unused: section 1 is the whole clip set.
pub fn build_subgraph_offsets_body(
    core_behavior_disk: &Path,
    core_behavior_rel: &str,
    meshes_root: &Path,
    sapt_chain: &[String],
    event_clip_names: &BTreeSet<String>,
) -> Option<Vec<u8>> {
    let mut section1: Vec<OffsetsClipNoMotion> = Vec::new();
    let mut section2: Vec<OffsetsMotion> = Vec::new();
    let mut any_motion = false;
    // Dedup section2 by anim path (a clip can be referenced by >1 generator).
    let mut seen_paths: BTreeSet<String> = BTreeSet::new();

    // 1) The core behavior's named clip generators → section1 (name→path) and section2
    //    (motion block), unconditionally, as CK does (a static-but-annotated attack like
    //    `Attack7` still gets a section2 entry).
    //
    //    Name identity is a format invariant; keep the guard. All 511064 section-1 entries in
    //    the 3156 vanilla files have `clip_name == basename(anim_path)`. The runtime
    //    (`0x1313BE0`) looks a clip up by generator name, then probes a per-subgraph table
    //    keyed by clip name with the record's animation basename, so an aliased entry
    //    (`AttackMelee_TuskSwipe_Front` over `TuskSwipe_Front.hkx`) is dropped in-game and
    //    `attackTime` stays 0.0. Aliased generators are repaired upstream (`align_clip_names`).
    //    The guard also drops dynamic clips, whose generator plays the `Idle.hkt` placeholder
    //    (real animation injected at runtime); CK excludes those too.
    for (clip_name, animation_name) in collect_clip_generators(core_behavior_disk) {
        // NOT gated on `event_clip_names`. Section 1 is the subgraph's whole clip set, not the
        // AI-action subset: restricting it to AnimEventInfo targets cost 110 distinct names
        // against the CK-built golden, `WPNIdleSighted` in all 47 subgraphs among them.
        let _ = &event_clip_names;
        let leaf = clip_leaf(&animation_name);
        // e.g. Actors\X\Animations\Foo.hkx
        let rel = resolve_leaf(
            meshes_root,
            sapt_chain,
            &leaf,
            core_project_dir(meshes_root, core_behavior_disk).as_deref(),
        );
        let anim_path_no_ext = rel.strip_suffix(".hkx").unwrap_or(&rel).to_string();
        if !clip_name_matches_animation(&clip_name, &anim_path_no_ext) {
            continue; // aliased or placeholder clip — unresolvable at runtime (see above)
        }
        section1.push(OffsetsClipNoMotion {
            clip_name: clip_name.clone(),
            anim_path: anim_path_no_ext.clone(),
        });
        if !seen_paths.insert(anim_path_no_ext.clone()) {
            continue;
        }
        let disk = meshes_root.join(rel.replace('\\', "/"));
        push_motion_entry(&disk, anim_path_no_ext, &mut section2, &mut any_motion);
    }

    // 2) Directional turn-in-place clips (`TurnLeft<deg>`/`TurnRight<deg>`) — driven by
    //    the engine turn-to-face system, not the behavior graph, so they are not clip
    //    generators; collect them on disk across the SAPT chain. section2-only.
    for (anim_path_no_ext, disk) in directional_turn_anims(meshes_root, sapt_chain) {
        if !seen_paths.insert(anim_path_no_ext.clone()) {
            continue;
        }
        push_motion_entry(&disk, anim_path_no_ext, &mut section2, &mut any_motion);
    }

    if !any_motion {
        return None;
    }
    section1.sort_by(|a, b| a.anim_path.cmp(&b.anim_path));
    section2.sort_by(|a, b| a.anim_path.cmp(&b.anim_path));
    Some(animation_offsets_populated_body(
        core_behavior_rel,
        &section1,
        &section2,
    ))
}

/// Basename (last `\`/`/` component) of an extension-less anim path, lowercased.
fn basename_lc(anim_no_ext: &str) -> String {
    anim_no_ext
        .replace('/', "\\")
        .rsplit('\\')
        .next()
        .unwrap_or(anim_no_ext)
        .to_ascii_lowercase()
}

/// The engine's on-disk locomotion transition clips (weapon_path.md): start/stop/turn-in-place
/// clips driven by the locomotion system, not the behavior graph (so not `hkbClipGenerator`s).
/// Generalizes the creature `is_directional_turn`: `stand_to_*`, `*_to_stand`, `*_to_idle`,
/// `turninplace*`, `relaxedturninplace*`. `_loop` hold variants are cyclic and belong to
/// SpeedInfo, not Offsets.
fn is_weapon_transition(stem: &str) -> bool {
    let lc = stem.to_ascii_lowercase();
    if lc.ends_with("_loop") {
        return false;
    }
    lc.starts_with("stand_to_")
        || lc.contains("_to_stand")
        || lc.contains("_to_idle")
        || lc.starts_with("turninplace")
        || lc.starts_with("relaxedturninplace")
}

/// Non-action clip families CK keeps out of a 3rd-person weapon subgraph's Offsets despite
/// root motion: hit reactions, staggers/stumbles, deaths/getups, cameras, jumps, equip/recoil,
/// cover crouch-walks (none appear in any FAN-oracle section 1). Applied only to 3rd-person
/// locomotion subgraphs; 1st-person/additive cores (no SpeedInfo) keep their full closure.
fn is_non_action_clip(base: &str) -> bool {
    base.starts_with("hit")
        || base.starts_with("riflehit")
        || base.contains("camerahit")
        || base.starts_with("stagger")
        || base.starts_with("raiderstumble")
        || base.contains("falldown")
        || base.starts_with("getup_")
        || base.starts_with("essentialdown")
        || base.contains("vatscrit")
        || base.starts_with("camera")
        || base.starts_with("pairedcamera")
        || base.starts_with("wpnjump")
        || base.starts_with("teleportationland")
        || (base.contains("coverright") && base.contains("walk"))
        || matches!(
            base,
            "swimidle"
                | "wpnrecoil"
                | "wpnequip"
                | "wpnunequip"
                | "wpnassemblypose"
                | "wpnassemblypose_left"
                | "riflepanicfire"
        )
}

/// Collect the on-disk locomotion transition clips across the SAPT chain, searched over
/// `roots` (mod first, then base) — the weapon analogue of `directional_turn_anims`.
/// Returns `(anim_path_no_ext_rel, disk_path)`; first dir+root that has a stem wins.
fn weapon_transition_anims(roots: &[&Path], sapt_chain: &[String]) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for sapt in sapt_chain {
        let dir_rel = sapt.trim_end_matches(['\r', '\n', ' ']);
        for root in roots {
            let disk_dir = root.join(dir_rel.replace('\\', "/"));
            let Ok(entries) = std::fs::read_dir(&disk_dir) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let is_hkx = p
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("hkx"));
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !is_hkx || !is_weapon_transition(stem) {
                    continue;
                }
                if seen.insert(stem.to_ascii_lowercase()) {
                    out.push((format!("{dir_rel}\\{stem}"), p));
                }
            }
        }
    }
    out
}

/// FO4's AnimationOffsets name-identity invariant: a section-1 row resolves at runtime only
/// when the clip generator's name equals the basename of the animation it plays. Shared by
/// the creature and weapon builders so the two cannot drift apart.
pub(super) fn clip_name_matches_animation(clip_name: &str, anim_path_no_ext: &str) -> bool {
    let base = anim_path_no_ext
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(anim_path_no_ext);
    base.eq_ignore_ascii_case(clip_name)
}

/// Build the weapon/character per-subgraph `AnimationOffsets/<id>.txt` (weapon_path.md).
///
/// Unlike the creature builder ([`build_subgraph_offsets_body`], single-file, mod-only), a
/// weapon subgraph's clips live across `[mod, base]` behind the base-game character graph:
/// the clip universe is the cross-file `GraphResolver` closure that `AnimationFileData` uses
/// (`collect_clip_generators(core)` finds 1 of 411 for a wrapping behavior), clips resolve
/// across `[mod, base]`, and the on-disk transition sweep covers the full weapon set
/// ([`weapon_transition_anims`]).
///
/// `speedinfo_loops` holds the subgraph's `AnimationSpeedInfo` contour leaf basenames
/// ([`super::speed::speed_info_leaf_basenames`]); those cyclic loops are excluded (moonwalk
/// guard). An empty set (no SpeedInfo, e.g. 1st-person `GunBehavior`) excludes nothing.
///
/// `section1` is the on-disk closure minus the loops and, for 3rd-person locomotion
/// subgraphs, the non-action families ([`is_non_action_clip`]). CK's finer action selection
/// within the rest (`WPNFireSingleAdditive` kept, `wpnfireautoadditive` dropped) depends on
/// graph-event reachability that the weapon oracle ships no AnimEventInfo to pin, so it is
/// not reproduced.
pub fn build_subgraph_offsets_body_weapon(
    resolver: &mut GraphResolver,
    core_behavior_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
    speedinfo_loops: &BTreeSet<String>,
) -> Option<Vec<u8>> {
    let mut section1: Vec<OffsetsClipNoMotion> = Vec::new();
    let mut section2: Vec<OffsetsMotion> = Vec::new();
    let mut any_motion = false;
    let mut seen_paths: BTreeSet<String> = BTreeSet::new();
    // 3rd-person locomotion subgraphs (own a SpeedInfo contour) apply CK's non-action drop;
    // 1st-person / additive cores (no contour) carry their full moving closure.
    let third_person = !speedinfo_loops.is_empty();

    // 1) Cross-file closure clips → section1 (clip→path) + section2 (motion block; empty lanes
    //    and the clip's own duration when it has no baked reference frame).
    // Weapon/creature section 1 keys on the generator's own `name`, byte-exact against CK.
    for (clip_name, _anim_basename, anim_no_ext, disk) in
        resolver.resolve_clip_generators(core_behavior_rel, sapt_chain)
    {
        let base = basename_lc(&anim_no_ext);
        if speedinfo_loops.contains(&base) {
            continue; // SpeedInfo's loop — excluding it here is the moonwalk guard
        }
        if third_person && is_non_action_clip(&base) {
            continue;
        }
        // Name-identity invariant, as in the creature path: the runtime resolves the clip's
        // animation through a table keyed by clip name, so an aliased row is unresolvable.
        // Vanilla always satisfies it (zero aliased rows in every vanilla SuperMutant
        // subgraph); converted FO76 humanoids reach this path through the shared
        // Weapon/Melee/MT cores and may not.
        //
        // Checked before the `seen_paths` insert: an aliased generator must not take the path's
        // one slot from a later well-named generator playing the same animation.
        if !clip_name_matches_animation(&clip_name, &anim_no_ext) {
            continue; // aliased clip — the engine cannot resolve it, so CK never writes it
        }
        if !seen_paths.insert(anim_no_ext.clone()) {
            continue;
        }
        section1.push(OffsetsClipNoMotion {
            clip_name,
            anim_path: anim_no_ext.clone(),
        });
        // Root motion decides the motion block, not registration: CK's section 1 includes
        // stationary clips (`WPNIdleSighted` in all 47 subgraphs of the CK-built golden,
        // `WPNIdleReady`, the cover/kneel idles, `SneakWPNFireSingleReady`).
        let reference_frame = extract_baked_reference_frame(&disk);
        let annotations = extract_root_annotations(&disk);
        let (duration, translations, rotations) = match reference_frame.as_deref() {
            Some(rf) => {
                any_motion |= frame_has_motion(rf);
                let (translations, rotations) = rf.reduced_lanes();
                (rf.duration, translations, rotations)
            }
            None => (
                extract_clip_duration(&disk).unwrap_or_default(),
                Vec::new(),
                Vec::new(),
            ),
        };
        section2.push(OffsetsMotion {
            anim_path: anim_no_ext,
            duration,
            translations,
            rotations,
            annotations,
        });
    }

    // 2) On-disk locomotion transitions (start/stop/turn-in-place) — section2-only, same as the
    //    creature directional-turn sweep but the full weapon set, across [mod, base].
    for (anim_no_ext, disk) in weapon_transition_anims(roots, sapt_chain) {
        let base = basename_lc(&anim_no_ext);
        if speedinfo_loops.contains(&base) {
            continue;
        }
        if !seen_paths.insert(anim_no_ext.clone()) {
            continue;
        }
        push_motion_entry(&disk, anim_no_ext, &mut section2, &mut any_motion);
    }

    if !any_motion {
        return None;
    }
    section1.sort_by(|a, b| a.anim_path.cmp(&b.anim_path));
    section2.sort_by(|a, b| a.anim_path.cmp(&b.anim_path));
    Some(animation_offsets_populated_body(
        core_behavior_rel,
        &section1,
        &section2,
    ))
}

/// True for the FO4 furniture behavior cores (`WorkbenchFurnitureBehavior`,
/// `FurnitureBehavior`, `FurnitureNoMirrorBehavior`, `1stPFurnitureIdleBehavior`,
/// `SingleAnimFurniture`, the furniture wrapping behaviors) plus `AmbushBehavior`,
/// the shared burrow/emerge core.
///
/// The RACE mounts `AmbushBehavior` in a Furniture-role block (FO76 floater; FO4 ships
/// vanilla offsets entries for it). In the weapon builder its motion gate emits no file and
/// the creature cannot build the subgraph. The real discriminator is the `SRAF` role; the
/// name match is a proxy because `SubgraphInput` does not carry the role.
pub fn is_furniture_core_behavior(core_behavior_rel: &str) -> bool {
    let lowercase = core_behavior_rel.to_ascii_lowercase();
    lowercase.contains("furniture") || lowercase.contains("ambushbehavior")
}

/// The clip's own duration, for clips that ship no baked reference frame.
fn clip_duration(clip_hkx: &Path) -> Option<f32> {
    let data = std::fs::read(clip_hkx).ok()?;
    let hkx = read_packfile(&data).ok()?;
    hkx.objects()
        .iter()
        .find(|o| o.class_name.starts_with("hka") && o.class_name.ends_with("Animation"))
        .and_then(|o| f32_member(o, "duration"))
}

/// Build the per-subgraph `AnimationOffsets/<id>.txt` body for a **furniture** subgraph.
///
/// Differs from the creature/weapon builders, per CK output:
/// 1. No motion gate. All 252 of FO4's Furniture-role subgraph blocks ship an offsets entry,
///    including static ones (`Furniture\Chair` and `Furniture\BarStool` each carry one clip
///    whose single translation sample is `(0,0,0)`); without the file the engine cannot
///    build the subgraph.
/// 2. Clips with no baked reference frame get a synthesized neutral frame. FO76 furniture
///    clips carry a null `extractedMotion` in the source itself, and CK's static-furniture
///    entries are exactly a neutral frame; dropping the clips would empty both sections.
///
/// The clip universe is the cross-file `GraphResolver` closure, since furniture cores live
/// in the base game.
pub fn build_subgraph_offsets_body_furniture(
    resolver: &mut GraphResolver,
    core_behavior_rel: &str,
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    let mut section1: Vec<OffsetsClipNoMotion> = Vec::new();
    let mut section2: Vec<OffsetsMotion> = Vec::new();
    let mut seen_paths: BTreeSet<String> = BTreeSet::new();

    // Furniture section 1 keys on the ANIMATION basename, not the generator's `name`. FO4's
    // shared furniture graph names a generator `Standing Enter` whose animation is
    // `EnterFromStand`; CK writes the latter, and `Standing Enter` appears in 0 of the 3156
    // vanilla AnimationOffsets files. Using the generator name makes GetClipInformation miss,
    // which removes the InteractionData entry and yields `sFailedActivation`.
    for (_generator_name, anim_basename, anim_no_ext, disk) in
        resolver.resolve_clip_generators(core_behavior_rel, sapt_chain)
    {
        if !seen_paths.insert(anim_no_ext.clone()) {
            continue;
        }
        section1.push(OffsetsClipNoMotion {
            clip_name: anim_basename,
            anim_path: anim_no_ext.clone(),
        });
        let (duration, translations, rotations) = match extract_baked_reference_frame(&disk) {
            Some(rf) => {
                let (translations, rotations) = rf.reduced_lanes();
                (rf.duration, translations, rotations)
            }
            None => {
                let duration = clip_duration(&disk).unwrap_or(1.0);
                (
                    duration,
                    vec![(duration, [0.0_f32; 3])],
                    vec![(duration, [0.0, 0.0, 0.0, 1.0])],
                )
            }
        };
        section2.push(OffsetsMotion {
            anim_path: anim_no_ext,
            duration,
            translations,
            rotations,
            annotations: extract_root_annotations(&disk),
        });
    }

    if section1.is_empty() {
        return None;
    }
    section1.sort_by(|a, b| a.anim_path.cmp(&b.anim_path));
    section2.sort_by(|a, b| a.anim_path.cmp(&b.anim_path));
    Some(animation_offsets_populated_body(
        core_behavior_rel,
        &section1,
        &section2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_motion_reduction_preserves_lanes_under_parallel_reuse() {
        let frame = BakedReferenceFrame {
            up: [-0.0, 0.0, 1.0],
            duration: 2.0,
            samples: (0..120)
                .map(|i| {
                    let t = i as f32 / 60.0;
                    [t, t * t, -0.0, t * 0.25]
                })
                .collect(),
            reduced_lanes: OnceLock::new(),
        };
        let expected = reduce_lanes(&frame);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let frame = &frame;
                let expected = &expected;
                scope.spawn(move || {
                    let actual = frame.reduced_lanes();
                    let bits = |lanes: &ReducedLanes| {
                        lanes
                            .0
                            .iter()
                            .flat_map(|(t, v)| std::iter::once(t).chain(v))
                            .chain(
                                lanes
                                    .1
                                    .iter()
                                    .flat_map(|(t, v)| std::iter::once(t).chain(v)),
                            )
                            .map(|v| v.to_bits())
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(bits(&actual), bits(expected));
                });
            }
        });
        assert!(frame.reduced_lanes.get().is_some());
    }

    #[test]
    #[ignore = "manual conversion performance benchmark"]
    fn benchmark_shared_motion_reduction() {
        let frame = BakedReferenceFrame {
            up: [0.0, 0.0, 1.0],
            duration: 10.0,
            samples: (0..600).map(|i| [i as f32, 0.0, 0.0, 0.0]).collect(),
            reduced_lanes: OnceLock::new(),
        };
        let started = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(reduce_lanes(std::hint::black_box(&frame)));
        }
        let uncached = started.elapsed();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(frame.reduced_lanes());
        }
        let cached = started.elapsed();
        assert_eq!(frame.reduced_lanes(), reduce_lanes(&frame));
        eprintln!("100 reductions, 600 samples: uncached={uncached:?}, cached including first reduction={cached:?}");
    }

    /// The identity rule both the creature and weapon builders gate on. It once existed only
    /// on the creature path, and the humanoids routing through the weapon path shipped 20533
    /// aliased rows the runtime cannot resolve.
    #[test]
    fn clip_name_identity_accepts_only_the_animations_own_basename() {
        // The shape CK ships: generator named for the animation it plays.
        assert!(clip_name_matches_animation(
            "SprintForward",
            r"Actors\MoleMiner\Animations\MT\sprintforward"
        ));
        // Case is not significant — the emitted paths are lowercased, the generators are not.
        assert!(clip_name_matches_animation(
            "AttackStandingA",
            r"Actors\MoleMiner\Animations\H2H\attackstandinga"
        ));
        // Only the leaf is compared; the directory chain must not participate.
        assert!(clip_name_matches_animation(
            "idle",
            r"Actors/Character/Animations/MT/idle"
        ));
        // A numeric-suffixed generator is the common FO76 alias and is NOT resolvable.
        assert!(!clip_name_matches_animation(
            "AttackStandingA01",
            r"Actors\MoleMiner\Animations\H2H\attackstandinga"
        ));
        // Nor is a genuinely renamed one.
        assert!(!clip_name_matches_animation(
            "MTJumpLandToRun",
            r"Actors\MoleMiner\Animations\MT\jumprunland"
        ));
        // A directory that happens to match the generator must not rescue a mismatched leaf.
        assert!(!clip_name_matches_animation(
            "MT",
            r"Actors\MoleMiner\Animations\MT\jumprunland"
        ));
    }

    /// The furniture builder must key section 1 on the animation basename. Rebuilding FO4's
    /// own `WorkbenchChemistryA` subgraph has CK's shipped file as the oracle: it contains
    /// `EnterFromStand` and no vanilla offsets file anywhere contains the generator name
    /// `Standing Enter`. Regressing to the generator name silently breaks activation.
    #[test]
    fn furniture_offsets_body_keys_section1_on_animation_basename() {
        let meshes =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4/Meshes");
        let core = r"Actors\Character\Behaviors\WorkbenchFurnitureBehavior.hkx";
        let sapt = r"Actors\Character\Animations\Furniture\WorkbenchChemistryA";
        if !meshes
            .join("Actors/Character/Animations/Furniture/WorkbenchChemistryA")
            .is_dir()
        {
            eprintln!("extracted WorkbenchChemistryA fixture absent; skipping");
            return;
        }
        let mut resolver = GraphResolver::new(vec![meshes]);
        let body = build_subgraph_offsets_body_furniture(&mut resolver, core, &[sapt.to_string()])
            .expect("chem furniture subgraph must produce an offsets body");

        let contains = |needle: &str| body.windows(needle.len()).any(|w| w == needle.as_bytes());
        assert!(
            contains("EnterFromStand"),
            "section 1 must key on the animation basename, as CK's own file does",
        );
        assert!(
            !contains("Standing Enter"),
            "generator name leaked into section 1 — this is the activation-breaking regression",
        );
    }

    /// Every FO4 furniture core must route to the furniture builder; creature and weapon
    /// cores must not, so their byte-exact paths keep owning their subgraphs.
    #[test]
    fn furniture_cores_are_recognised_and_others_are_not() {
        for core in [
            r"Actors\Character\Behaviors\WorkbenchFurnitureBehavior.hkx",
            r"Actors\Character\Behaviors\FurnitureBehavior.hkx",
            r"Actors\Character\Behaviors\FurnitureNoMirrorBehavior.hkx",
            r"Actors\Character\Behaviors\SingleAnimFurniture.hkx",
            r"Actors\Character\_1stPerson\Behaviors\1stPFurnitureIdleBehavior.hkx",
            r"Actors\Character\Behaviors\UseBodyMorphOffsetFurnitureWrappingBehavior.hkx",
            r"Actors\Character\Behaviors\EnableSneakFurnitureWrappingBehavior.hkx",
            // Furniture-role block, no "furniture" in the path.
            r"Actors\Shared\Behaviors\AmbushBehavior.hkx",
        ] {
            assert!(is_furniture_core_behavior(core), "{core}");
        }
        for core in [
            r"Actors\Snallygaster\Behaviors\SnallygasterCoreBehavior.hkx",
            r"Actors\Character\Behaviors\GunBehavior.hkx",
            r"Actors\Character\_1stPerson\Behaviors\1stPGunBehavior.hkx",
        ] {
            assert!(!is_furniture_core_behavior(core), "{core}");
        }
    }

    fn aggregate_subgraph(core_behavior: &str, sapt_chain: &[&str]) -> SubgraphInput {
        SubgraphInput {
            core_behavior: core_behavior.to_string(),
            sapt_chain: sapt_chain.iter().map(|path| (*path).to_string()).collect(),
            race_dir: None,
        }
    }

    #[test]
    fn offsets_aggregate_candidate_rejects_injured_core_and_sapt_paths() {
        assert!(is_aggregate_candidate(&aggregate_subgraph(
            r"Actors\Character\Behaviors\WeaponBehavior.hkx",
            &[r"Actors\Character\Animations\Weapon\Pistol"],
        )));
        assert!(!is_aggregate_candidate(&aggregate_subgraph(
            r"Actors\Character\Behaviors\INJUREDWeaponBehavior.hkx",
            &[r"Actors\Character\Animations\Weapon\Pistol"],
        )));
        assert!(!is_aggregate_candidate(&aggregate_subgraph(
            r"Actors\Character\Behaviors\WeaponBehavior.hkx",
            &[r"Actors\Character\Animations\Weapon\Pistol\InJuReD\Left"],
        )));
    }

    #[test]
    fn offsets_aggregate_removes_exact_vanilla_injured_exclusions() {
        const EXPECTED: &[u64] = &[
            2798016766296788894,
            2815779307924033438,
            2850019573186760606,
            3634740580962310295,
            5737393547891773342,
            7308697184125652894,
            9004150397693398942,
            9331636681854720151,
            12717401627996524446,
        ];
        assert_eq!(VANILLA_INJURED_EXCLUSIONS, EXPECTED);

        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("aggregate.txt");
        let mut content = EXPECTED
            .iter()
            .map(|id| format!("{id}.txt\n"))
            .collect::<String>();
        let retained_ids: BTreeSet<u64> = EXPECTED
            .iter()
            .map(|id| id + 1)
            .chain(std::iter::once(41))
            .collect();
        content.extend(retained_ids.iter().map(|id| format!("{id}.txt\n")));
        std::fs::write(&base, content).unwrap();

        assert_eq!(
            build_offsets_aggregate(&[], Some(&base)),
            Some(
                retained_ids
                    .into_iter()
                    .map(|id| format!("{id}.txt\n"))
                    .collect()
            )
        );
    }

    #[test]
    fn offsets_aggregate_is_sorted_deduplicated_and_newline_terminated() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("aggregate.txt");
        std::fs::write(&base, "30.txt\n2.txt\n30.txt\n").unwrap();
        let subgraphs = vec![
            aggregate_subgraph(
                r"Actors\Character\Behaviors\WeaponBehavior.hkx",
                &[r"Actors\Character\Animations\Weapon\Pistol"],
            ),
            aggregate_subgraph(
                r"Actors\Character\Behaviors\WeaponBehavior.hkx",
                &[r"Actors\Character\Animations\Weapon\Pistol"],
            ),
        ];

        let lines = build_offsets_aggregate(&subgraphs, Some(&base)).unwrap();
        let expected_ids: BTreeSet<u64> = [2, 30, subgraphs[0].id()].into_iter().collect();
        let expected: Vec<String> = expected_ids
            .into_iter()
            .map(|id| format!("{id}.txt\n"))
            .collect();
        assert_eq!(lines, expected);

        let body = lines.concat();
        assert!(body.ends_with('\n'));
        assert!(!body.ends_with("\n\n"));
    }

    #[test]
    fn offsets_aggregate_requires_readable_valid_base() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.txt");
        assert!(build_offsets_aggregate(&[], None).is_none());
        assert!(build_offsets_aggregate(&[], Some(&missing)).is_none());
        assert!(build_offsets_aggregate(&[], Some(temp.path())).is_none());

        let base = temp.path().join("aggregate.txt");
        std::fs::write(&base, "").unwrap();
        assert!(build_offsets_aggregate(&[], Some(&base)).is_none());

        for malformed in [
            "not-an-id.txt",
            "8",
            "9.bin",
            "10.txt.txt",
            "18446744073709551616.txt",
            "13.TXT",
            "folder/14.txt",
            "folder\\15.txt",
        ] {
            std::fs::write(&base, format!("7.txt\n{malformed}\n12.txt\n")).unwrap();
            assert!(
                build_offsets_aggregate(&[], Some(&base)).is_none(),
                "accepted malformed aggregate entry {malformed:?}"
            );
        }
    }
}
