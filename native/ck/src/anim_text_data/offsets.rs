//! AnimationOffsets per-subgraph populated emission — root-motion fixup (CK-free).
//!
//! The engine treats a present-but-EMPTY `AnimationOffsets/<id>.txt` as authoritative
//! ("this subgraph has no root motion"), which kills forward locomotion / turn
//! displacement for converted creatures (sliding / moonwalking). So a **moving** subgraph
//! must ship a populated per-subgraph file with non-empty translation/rotation blocks.

//!
//! ## Source (RE: `bucket5_offsets_findings.md`, `reframe.py`)
//! Root motion is the clip's baked `hkaDefaultAnimatedReferenceFrame` — NOT the root bone
//! channel (`root_motion.rs::extract_from_root_channel` reads the wrong source). Each
//! `referenceFrameSamples[i]` is a `Vector4 (x,y,z,w)` where `(x,y,z)` is the per-frame
//! world displacement and `w` = `paraDistance` = accumulated heading angle in radians.
//! The Offsets rotation key is the axis-angle quaternion about the reference-frame `up`:
//! `q = (up.x·sin(θ/2), up.y·sin(θ/2), up.z·sin(θ/2), cos(θ/2))`, `θ = w`. Read from the
//! **binary** `HkxValue` (true f32) — `unpack_hkx_to_xml`'s `%f` text truncates ~1 ULP.
//! Signed zeros fall out of the plain f32 multiply (right turns: `0.0·(−s) = −0.0`).
//!
//! ## Keyframe reduction
//! `reduce_lanes` reduces the dense per-frame samples to a sparse keyset via `grow_while_fits`
//! (selection/count/time byte-exact vs CK, `offsets_byte_exact.md`; sampled values within
//! ≤1 ULP). The engine interpolates linearly between the kept keys.
//!
//! ## Clip set (RE: validated against the CK Snallygaster oracle file-for-file)
//! The offsets clip set is the AI-action subset — NOT the AnimationFileData closure
//! (which also lists locomotion/idle/reaction clips that CK keeps OUT of offsets):
//! * `section1` (`clip_name` → `anim_path`, no motion block) = the **AnimEventInfo
//!   non-dynamic clip targets** (combat attacks/evades/fire). Proven equal to the CK
//!   section-1 set; the only AnimEventInfo clip dropped is the dynamic `DynamicAnimA`,
//!   which has no on-disk anim (the clip-generator scan already requires a non-empty
//!   `animationName`).
//! * `section2` (`anim_path` → root-motion block) = the section-1 clips **plus** the
//!   engine's directional turn-in-place clips (`TurnLeft<deg>`/`TurnRight<deg>` found on
//!   disk; the plain `TurnLeft`/`TurnRight` are excluded). The directional turns are
//!   driven by the engine turn-to-face system, not the behavior graph, so they are NOT
//!   `hkbClipGenerator`s — they are collected by on-disk name pattern across the SAPT
//!   chain. Locomotion (walk/run/jog), idle, hit-reactions, staggers and deaths are
//!   excluded (CK does not put them in this cache; the locomotion system owns their
//!   root motion). The byte-exact grammar is `offsets_struct.py` (3159/3162 re-emit).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use havok_native::hkx::HkxObject;
use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;

use super::behavior_index::{clip_leaf, resolve_leaf};
use super::bucket_files::{OffsetsClipNoMotion, OffsetsMotion, animation_offsets_populated_body};
use super::emit::SubgraphInput;
use super::graph::GraphResolver;

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

/// Read the baked reference frame from a clip `.hkx` (binary model, true f32).
pub(super) fn extract_baked_reference_frame(clip_hkx: &Path) -> Option<BakedReferenceFrame> {
    let data = std::fs::read(clip_hkx).ok()?;
    let hkx = read_packfile(&data).ok()?;
    let obj = hkx
        .objects()
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
    })
}

/// Streaming **grow-while-fits** piecewise-linear decimation. Returns the kept frame
/// indices: frame 0 is the IMPLICIT origin (NEVER emitted) and the terminal frame `N-1` is
/// always retained. `err(a,b,j)` = the error of dropping interior frame `j` from the chord
/// `a→b`; a segment `[a,b]` fits iff every interior `j` has `err ≤ tol`. (RE:
/// `offsets_byte_exact.md` §1 — this is NOT RDP and NOT furthest-reachable.)
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
/// SELECTION + COUNT + TIME are byte-exact vs CK (`offsets_byte_exact.md`):
/// * translation — grow-while-fits, Euclidean-3D chord error, `tol = 2.0`.
/// * rotation — grow-while-fits, heading-angle error `|Δw|`, `tol = 0.3°`, plus the antipode
///   rule (a segment spanning ≥180° — `dot(q_a,q_b) ≤ 0` — forces a key at the last frame
///   before `qw` goes negative; disambiguates ±180° turns, gated on the f32 `qw` sign so
///   `+π` TurnLeft180 forces a key but `−π` TurnRight180 does not).
/// * time — `t[fr] = f32(fr · f32(duration/(N-1)))` (NOT `fr/fps`, which is 1 ULP off).
/// * values — sampled from the reference frame at the key time via Havok `getReferenceFrame`
///   interpolation (§4), NOT a verbatim `samples[fr]` read → ≤1 ULP (the last ULP is the
///   exact f32 getter widths/op-order, an accepted refinement). SELECTION still runs on the
///   verbatim dense samples.
fn reduce_lanes(rf: &BakedReferenceFrame) -> (Vec<(f32, [f32; 3])>, Vec<(f32, [f32; 4])>) {
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

    // VALUE source = Havok `getReferenceFrame(t)` sampled at the key time, NOT the verbatim
    // `samples[fr]` (offsets_byte_exact.md §4). The f32 `time→p` round-trip lands `p` a hair
    // off the integer, so the lerp nudges an on-frame key by ≤1 ULP — exactly what CK stores.
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
    let Ok(data) = std::fs::read(clip_hkx) else {
        return Vec::new();
    };
    let Ok(hkx) = read_packfile(&data) else {
        return Vec::new();
    };
    for obj in hkx.objects() {
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
    let Ok(data) = std::fs::read(behavior) else {
        return Vec::new();
    };
    let Ok(hkx) = read_packfile(&data) else {
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
    let (translations, rotations) = reduce_lanes(&rf);
    section2.push(OffsetsMotion {
        anim_path: anim_path_no_ext,
        duration: rf.duration,
        translations,
        rotations,
        annotations,
    });
}

/// Build the **populated** per-subgraph `AnimationOffsets/<id>.txt` body for one subgraph,
/// or `None` if no clip in the subgraph actually moves (then the engine rebuilds offsets
/// from the empty project-level entry — correct for a genuinely static subgraph).
///
/// * `core_behavior_disk` — the subgraph's core behavior `.hkx` on disk (clip source).
/// * `core_behavior_rel`  — the FO4 relpath written as the V4 `core_behavior` string.
/// * `meshes_root`        — mod Meshes root (clip `.hkx` + SAPT override resolution).
/// * `sapt_chain`         — the subgraph SAPT chain, self-first.
/// * `event_clip_names`   — AnimEventInfo clip targets (ci) → the section1 named-clip set.
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

    // 1) Combat clips = the AnimEventInfo non-dynamic clip targets, resolved from the
    //    core behavior's named clip generators. Each → section1 (name→path) AND section2
    //    (motion block) — unconditionally, matching CK (a static-but-annotated attack
    //    like `Attack7` still gets a section2 entry).
    //
    //    The dynamic clip `DynamicAnimA` IS an AnimEventInfo target, but its generator
    //    plays the `Idle.hkt` *placeholder* (its real animation is runtime-injected), so
    //    it has no static root motion and CK drops it from this cache. Distinguish it
    //    structurally: a real named combat clip plays its OWN animation (`Attack6` →
    //    `Attack6.hkx`); a dynamic/aliased clip's resolved anim basename differs from its
    //    name.
    for (clip_name, animation_name) in collect_clip_generators(core_behavior_disk) {
        if !event_clip_names.contains(&clip_name.to_ascii_lowercase()) {
            continue; // not an AI-action combat clip — CK keeps it out of offsets
        }
        let leaf = clip_leaf(&animation_name);
        let rel = resolve_leaf(meshes_root, sapt_chain, &leaf); // e.g. Actors\X\Animations\Foo.hkx
        let anim_path_no_ext = rel.strip_suffix(".hkx").unwrap_or(&rel).to_string();
        let anim_base = anim_path_no_ext
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&anim_path_no_ext);
        if !anim_base.eq_ignore_ascii_case(&clip_name) {
            continue; // dynamic/placeholder clip (e.g. DynamicAnimA → Idle) — no offset
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

/// The engine's on-disk locomotion **transition** clips (weapon_path.md §6b.2): the
/// start/stop/turn-in-place clips driven by the locomotion system, not the behavior graph
/// (so not `hkbClipGenerator`s). Generalizes the creature `is_directional_turn`:
/// `stand_to_*`, `*_to_stand`, `*_to_idle`, `turninplace*`, `relaxedturninplace*`. The `_loop`
/// hold variants are EXCLUDED (those are cyclic — SpeedInfo's, not Offsets').
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

/// Non-action clip families that CK keeps OUT of a 3rd-person weapon subgraph's Offsets even
/// though they carry root motion: hit reactions, staggers/stumbles, deaths/getups, cameras,
/// jumps, equip/recoil, cover crouch-walks. (Calibrated against the FAN oracle: 0 of these
/// appear in any section-1; weapon_path.md §6b.5.) Applied ONLY to 3rd-person locomotion
/// subgraphs — 1st-person/additive cores (no SpeedInfo) keep their full closure.
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

/// Build the weapon/character per-subgraph `AnimationOffsets/<id>.txt` (weapon_path.md §6b).
///
/// The creature builder ([`build_subgraph_offsets_body`]) is single-file + mod-only; a weapon
/// subgraph's clips live across `[mod, base]` behind the base-game character behavior graph. So
/// this path differs in three ways: (1) the clip universe is the cross-file `GraphResolver`
/// closure (the SAME closure `AnimationFileData` uses — `collect_clip_generators(core)` finds 1
/// of 411 for a wrapping behavior); (2) clips resolve across `[mod, base]`; (3) the on-disk
/// transition sweep covers the full weapon set ([`weapon_transition_anims`]).
///
/// `speedinfo_loops` = the subgraph's `AnimationSpeedInfo` directional-contour leaf basenames
/// ([`super::speed::speed_info_leaf_basenames`]). Those cyclic loops are
/// EXCLUDED (moonwalk guard); the exclusion is conditional — an empty set (a subgraph with no
/// SpeedInfo, e.g. 1st-person `GunBehavior`) drops nothing, so its loops stay in Offsets.
///
/// `section1` is the on-disk closure named set minus the loops and (for a 3rd-person locomotion
/// subgraph) the non-action families ([`is_non_action_clip`]). The exact action sub-selection
/// the human CK pass makes within the remainder (e.g. `WPNFireSingleAdditive` kept but
/// `wpnfireautoadditive` dropped) is graph-event-reachability the weapon oracle ships no
/// AnimEventInfo to pin — a documented byte-parity residual (weapon_path.md §6b.5). Dense-vs-CK-
/// keyframe-reduction caveat carries over from the creature path (functionally exact, larger).
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

    // 1) Cross-file closure clips → section1 (clip→path) + section2 (motion). A clip enters
    //    only if it has a baked reference frame (matches CK's s1 ⊆ s2; pure additive/pose clips
    //    with no root frame are dropped here, exactly as they are absent from the oracle).
    for (clip_name, anim_no_ext, disk) in
        resolver.resolve_clip_generators(core_behavior_rel, sapt_chain)
    {
        let base = basename_lc(&anim_no_ext);
        if speedinfo_loops.contains(&base) {
            continue; // SpeedInfo's loop — excluding it here is the moonwalk guard
        }
        if third_person && is_non_action_clip(&base) {
            continue;
        }
        let Some(rf) = extract_baked_reference_frame(&disk) else {
            continue;
        };
        if !seen_paths.insert(anim_no_ext.clone()) {
            continue;
        }
        section1.push(OffsetsClipNoMotion {
            clip_name,
            anim_path: anim_no_ext.clone(),
        });
        any_motion |= frame_has_motion(&rf);
        let annotations = extract_root_annotations(&disk);
        let (translations, rotations) = reduce_lanes(&rf);
        section2.push(OffsetsMotion {
            anim_path: anim_no_ext,
            duration: rf.duration,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregate_subgraph(core_behavior: &str, sapt_chain: &[&str]) -> SubgraphInput {
        SubgraphInput {
            core_behavior: core_behavior.to_string(),
            sapt_chain: sapt_chain.iter().map(|path| (*path).to_string()).collect(),
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
