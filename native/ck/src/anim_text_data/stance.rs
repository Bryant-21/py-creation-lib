//! AnimationStanceData (count=1 creature) — the camera-framing stance pose (CK-free).
//!
//! Every self-contained FO76→FO4 creature emits the **count=1** form: one stored pose
//! holding the **Head** and a **torso/spine camera-pivot** bone in model space at frame 0
//! of the subgraph's idle/standing clip, plus an IDENTITY third slot (RE: `stance_deep.md`
//! "REFINED + CONCLUSIVE container spec"; count>1 is base-game-`Character`-only). The
//! per-bone source is a DIRECT model-space frame-0 pose extraction — no IK / retarget /
//! solve. This mirrors the byte-exact-proven `stance_work/reemit.py` (MirelurkKing 124 B
//! container byte-identical; bone floats to the `hkaPose` accumulation residual):
//!
//! 1. load the creature skeleton (`characterassets/skeleton.hkx`) → reference local pose;
//! 2. extract the idle clip; overlay each animated track's **frame-0** local transform
//!    (track→bone via `transformTrackToBoneIndices`, else identity);
//! 3. accumulate local→model with the canonical `havok_native::animation::pose::Pose`
//!    (the `hkaPose` order — float-exactness needs this, do NOT hand-roll compose);
//! 4. write slot0 = Head, slot1 = torso pivot (quat W-FIRST), slot2 = IDENTITY.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use havok_native::animation::pose::{Pose, PoseSkeleton, QsTransform, quat_normalize};
use havok_native::animation::{SkeletonRecord, extract_clip, parse_skeleton_xml};
use havok_native::api::havok_hkx_to_xml;

use super::bucket_files::{
    AnimationStanceData, StanceDataCodecError, StancePose, StanceSec2Record, StanceTransform,
    animation_stance_data_count1_body, animation_stance_data_headtrack_body,
    animation_stance_data_multipose_body, decode_animation_stance_data_body, stance_sec2_tag,
};
use super::graph::{GraphResolver, PoseRoleProvenance, PoseRoleSourceKey, StancePerspective};

/// One stored stance slot: quaternion W-FIRST (w,x,y,z) + translation (x,y,z).
type Slot = ([f32; 4], [f32; 3]);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StanceFormKey {
    pub plugin: String,
    pub local: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponRaceFamily {
    pub owner_race: StanceFormKey,
    pub sadd: Option<StanceFormKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeaponSraf {
    pub role: u16,
    pub perspective: u16,
}

/// Rich RACE subgraph metadata supplied by the future emitter adapter. Keyword
/// order and the full authored SAPT list are intentionally retained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponSubgraphMetadata {
    pub race_family: WeaponRaceFamily,
    pub perspective: StancePerspective,
    pub sakd: Vec<StanceFormKey>,
    pub stkd: Vec<StanceFormKey>,
    pub core_behavior: String,
    pub sapt: Vec<String>,
    pub sraf: WeaponSraf,
    pub id: u64,
}

pub struct WeaponStanceRequest<'a> {
    pub target: &'a WeaponSubgraphMetadata,
    /// Base-game family subgraphs with stable ids. These are metadata, not a
    /// donor-id cache: every candidate is re-proven through its behavior graph.
    pub base_family_subgraphs: &'a [WeaponSubgraphMetadata],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StanceDonorSelection {
    pub pose_idx: u8,
    pub variant: u8,
    pub tag: u32,
    pub donor_id: u64,
    pub source: PoseRoleSourceKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StanceProceduralFallback {
    pub pose_idx: u8,
    pub variant: u8,
    pub tags: Vec<u32>,
    pub reason: ProceduralFallbackReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StanceGridProvenance {
    pub donor_records: Vec<StanceDonorSelection>,
    pub procedural_fallbacks: Vec<StanceProceduralFallback>,
    pub donor_record_count: usize,
    pub procedural_record_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProceduralFallbackReason {
    RoleTraversal(String),
    MissingSourceProvenance {
        pose_idx: u8,
        variant: u8,
    },
    MissingDonor {
        pose_idx: u8,
        variant: u8,
    },
    AmbiguousDonor {
        pose_idx: u8,
        variant: u8,
        donor_ids: Vec<u64>,
    },
    MissingDonorTags {
        pose_idx: u8,
        variant: u8,
        donor_id: u64,
        tags: Vec<u32>,
    },
    DonorShape {
        pose_idx: u8,
        variant: u8,
        donor_id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponGridSource {
    NotApplicable,
    Sidestep(StanceGridProvenance),
    Procedural(ProceduralFallbackReason),
}

#[derive(Debug, Clone)]
pub struct WeaponStanceBuild {
    pub body: Vec<u8>,
    pub pose_roles: Vec<PoseRoleProvenance>,
    pub grid_source: WeaponGridSource,
}

#[derive(Debug, thiserror::Error)]
pub enum WeaponStanceBuildError {
    #[error("invalid weapon stance metadata: {0}")]
    InvalidMetadata(String),
    #[error("character skeleton was not found on the supplied roots")]
    SkeletonMissing,
    #[error("character skeleton could not be decoded")]
    SkeletonDecode,
    #[error("character skeleton does not expose the required Head/Chest/Spine stance chain")]
    SkeletonBonesMissing,
    #[error("failed to sample stance clip {0:?}")]
    PoseClipDecode(PathBuf),
    #[error("failed to read base donor {id} at {path:?}: {source}")]
    DonorRead {
        id: u64,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("base donor {id} is not valid AnimationStanceData: {source}")]
    DonorDecode {
        id: u64,
        #[source]
        source: StanceDataCodecError,
    },
    #[error(transparent)]
    Codec(#[from] StanceDataCodecError),
}

/// `QsTransform` rotation is stored (x,y,z,w); the stance file stores (w,x,y,z).
fn to_wxyz(rot_xyzw: [f32; 4]) -> [f32; 4] {
    [rot_xyzw[3], rot_xyzw[0], rot_xyzw[1], rot_xyzw[2]]
}

/// Convert a model-space `QsTransform` to a stored stance slot (quat W-FIRST + trans).
fn slot_of(m: &QsTransform) -> Slot {
    (to_wxyz(m.rotation), m.translation)
}

/// Strip a leading skeleton side/namespace prefix (`C_`/`L_`/`R_`, and defensively
/// `Camera`/`Bip01`/`NPC `) so a converted creature's `C_Head`/`C_Spine4` bones match
/// the bare `Head`/`Spine*` selectors. The C_-prefix was the selector blocker on
/// FO76→FO4 converted skeletons (RE: `stance_converted_174b.md`).
fn strip_side_prefix(name: &str) -> &str {
    for p in ["C_", "L_", "R_", "Camera", "Bip01", "NPC "] {
        if let Some(rest) = name.strip_prefix(p) {
            return rest;
        }
    }
    name
}

/// Prefix-stripped, case-insensitive bone-name equality (`C_Head` == `Head`).
fn stripped_eq(name: &str, target: &str) -> bool {
    strip_side_prefix(name).eq_ignore_ascii_case(target)
}

/// Prefix-stripped, case-insensitive "starts with" (`C_Neck1` starts with `Neck`).
fn stripped_starts_with(name: &str, prefix: &str) -> bool {
    let s = strip_side_prefix(name);
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Head-local look-at offset for the section-2 head-track reference (RE:
/// `stance_converted_174b.md` §6.2 — derived from the Snallygaster level-head oracle
/// files; reproduces the look-at target to a few units). The engine recomputes the
/// real head-track at runtime, so this approximation degrades gracefully.
const HEADTRACK_LOCAL_T: [f32; 3] = [53.0, -7.0, 1.5];
const HEADTRACK_LOCAL_Q_XYZW: [f32; 4] = [-0.07, 0.0, -0.25, 0.965];

/// Compute the section-2 head-track / look-at reference as the head's model-space
/// frame-0 transform composed with the fixed head-local offset, returned as a stored
/// slot (quat W-FIRST + translation).
fn headtrack_lookat(head_model: &QsTransform) -> Slot {
    let local = QsTransform {
        translation: HEADTRACK_LOCAL_T,
        rotation: quat_normalize(&HEADTRACK_LOCAL_Q_XYZW),
        scale: [1.0, 1.0, 1.0],
    };
    let m = QsTransform::compose(head_model, &local);
    // CK stores a unit quaternion; renormalize the composed rotation (the recipe's
    // approximate local quat is not exactly unit, and accumulation drifts).
    (to_wxyz(quat_normalize(&m.rotation)), m.translation)
}

/// Whether the core behavior declares head-tracking (`bGraphWantsHeadTracking` /
/// `isActiveModifier_HeadTracking`). This is the gate that distinguishes the 174 B
/// converted-creature StanceData (`sec2_count=1`) from the 124 B vanilla form (RE:
/// `stance_converted_174b.md` §6). The signals are inline strings in the behavior
/// packfile, so a raw byte scan is sufficient and avoids a full graph parse.
pub fn behavior_wants_head_tracking(core_behavior_file: &Path) -> bool {
    let Ok(data) = std::fs::read(core_behavior_file) else {
        return false;
    };
    const SIGS: [&[u8]; 2] = [b"bGraphWantsHeadTracking", b"isActiveModifier_HeadTracking"];
    SIGS.iter()
        .any(|sig| data.windows(sig.len()).any(|w| w == *sig))
}

/// Parse a creature skeleton `.hkx` into `(record, pose-skeleton)`. The pose skeleton's
/// reference-local transforms seed the per-frame overlay. Returns `None` if the file is
/// unreadable or carries no reference pose.
fn load_skeleton(skeleton_file: &Path) -> Option<(SkeletonRecord, PoseSkeleton)> {
    let data = std::fs::read(skeleton_file).ok()?;
    let xml = havok_hkx_to_xml(&data).ok()?;
    let skd = parse_skeleton_xml(&xml).ok()?;
    let n = skd.bone_names.len();
    if n == 0 || skd.reference_pose.len() != n || skd.parent_indices.len() != n {
        return None;
    }
    let reference_local: Vec<QsTransform> = skd
        .reference_pose
        .iter()
        .map(|b| QsTransform {
            translation: b.t,
            rotation: b.q, // xyzw, as the parser stores it
            scale: b.s,
        })
        .collect();
    let skeleton = PoseSkeleton {
        bone_names: skd.bone_names.clone(),
        parent_indices: skd.parent_indices.clone(),
        reference_local,
    };
    Some((skd, skeleton))
}

/// Sample the model-space frame-0 transforms of `head`/`torso` from the idle clip,
/// returning the raw `QsTransform`s (rotation xyzw). The clip's frame-0 local transforms
/// overlay the skeleton reference pose; bones the clip does not animate keep their
/// reference local. Uses the canonical `Pose` local→model accumulation. Raw transforms
/// (not pre-converted slots) so the caller can compose the head with the head-track
/// look-at offset for section-2.
fn sample_models(
    skel: &SkeletonRecord,
    pose_skel: &PoseSkeleton,
    clip_file: &Path,
    head: usize,
    torso: usize,
) -> Option<(QsTransform, QsTransform)> {
    let mut pose = sample_model_pose(skel, pose_skel, Some(clip_file))?;
    Some((pose.model_at(head), pose.model_at(torso)))
}

fn sample_model_pose(
    skel: &SkeletonRecord,
    pose_skel: &PoseSkeleton,
    clip_file: Option<&Path>,
) -> Option<Pose> {
    let Some(clip_file) = clip_file else {
        return Some(Pose::from_local(
            pose_skel.clone(),
            pose_skel.reference_local.clone(),
        ));
    };
    let data = std::fs::read(clip_file).ok()?;
    let xml = havok_hkx_to_xml(&data).ok()?;
    let clip = extract_clip(&xml, Some(skel)).ok()?;

    let mut local = pose_skel.reference_local.clone();
    let n = local.len();
    for (ti, ch) in clip.channels.iter().enumerate() {
        // Track → bone: the binding remap, else by name (identity), else the track index.
        let bi = clip
            .track_to_bone_indices
            .get(ti)
            .map(|x| *x as usize)
            .or_else(|| pose_skel.bone_names.iter().position(|b| *b == ch.bone_name))
            .unwrap_or(ti);
        if bi >= n {
            continue;
        }
        if let Some(r) = ch.rotations.first() {
            local[bi].rotation = r.value;
        }
        if let Some(t) = ch.translations.first() {
            local[bi].translation = t.value;
        }
    }

    Some(Pose::from_local(pose_skel.clone(), local))
}

/// Index of the first bone whose name equals `target` (case-insensitive).
fn find_bone(names: &[String], target: &str) -> Option<usize> {
    names.iter().position(|n| n.eq_ignore_ascii_case(target))
}

/// Select the (head, spine-pivot) bone pair for a creature skeleton. slot0 = the **Head**
/// (or `HeadTwist`) bone, matched prefix-tolerantly (`C_Head`). slot1 = the **parent of
/// the first neck bone** — topologically the first ancestor of the head that is neither a
/// neck nor a head bone (e.g. `C_Spine4`, parent of `C_Neck1`). This unifies the vanilla
/// cases (MirelurkKing `Spine1`, MoleRat `Spine2`, LibertyPrime/SentryBot `Chest` are all
/// "parent of the first neck"), and the prefix-tolerant matching is what lets it pick the
/// right bone on a converted `C_`-prefixed skeleton (RE: `stance_converted_174b.md` §3).
fn select_head_torso(skel: &PoseSkeleton) -> Option<(usize, usize)> {
    // Tier 1 — exact `Head`/`C_Head` selection. Every vanilla and simply-`C_`-prefixed
    // skeleton (MirelurkKing, MoleRat, LibertyPrime, MegaSloth, Snallygaster, ...) has an
    // exact head bone and is resolved here byte-for-byte as before. Tier 2 runs only when
    // this finds nothing, so those cases are provably unchanged.
    select_head_torso_exact(skel).or_else(|| select_head_torso_core(skel))
}

fn select_head_torso_exact(skel: &PoseSkeleton) -> Option<(usize, usize)> {
    let head = (0..skel.bone_names.len()).find(|&i| {
        stripped_eq(&skel.bone_names[i], "Head") || stripped_eq(&skel.bone_names[i], "HeadTwist")
    })?;
    // Ancestor chain head -> ... -> root.
    let mut ancestors: Vec<usize> = Vec::new();
    let mut cur = skel.parent_indices[head];
    while cur >= 0 {
        let a = cur as usize;
        ancestors.push(a);
        cur = skel.parent_indices[a];
    }
    // Topological pivot: the first ancestor that is neither a neck nor a head bone =
    // parent of the base-of-neck (the validated EXACT pivot). Fall back to the legacy
    // name-priority over ancestors, then the immediate parent — never the head itself.
    let torso = ancestors
        .iter()
        .copied()
        .find(|&a| {
            let n = &skel.bone_names[a];
            !stripped_starts_with(n, "Neck") && !stripped_starts_with(n, "Head")
        })
        .or_else(|| {
            ["Spine2", "Spine1", "Chest", "Hub", "Spine", "Pelvis", "COM"]
                .iter()
                .find_map(|p| {
                    ancestors
                        .iter()
                        .copied()
                        .find(|&a| stripped_eq(&skel.bone_names[a], p))
                })
        })
        .or_else(|| ancestors.first().copied())?;
    Some((head, torso))
}

/// Reduce a bone name to its semantic token: drop everything up to and including the last
/// leading `C_`/`L_`/`R_` side marker (covering custom namespaces — `Mothman_BN_C_Head`,
/// `HB_C_Head`, `jnt_C_head`), then strip surrounding digits (`C_00Head1`/`C_head00` ->
/// `Head`). Converted FO76 creature skeletons carry these author prefixes that the plain
/// `strip_side_prefix` (C_/L_/R_ only) cannot see (RE: `grafton-animtext-three-emitter-defects`).
fn bone_core(name: &str) -> &str {
    let bytes = name.as_bytes();
    let mut cut = 0usize;
    for i in 0..name.len() {
        if i == 0 || bytes[i - 1] == b'_' {
            for m in ["C_", "L_", "R_"] {
                if name[i..].starts_with(m) {
                    cut = i + m.len();
                }
            }
        }
    }
    name[cut..].trim_matches(|c: char| c.is_ascii_digit())
}

fn core_eq(name: &str, target: &str) -> bool {
    bone_core(name).eq_ignore_ascii_case(target)
}

fn core_starts_with(name: &str, prefix: &str) -> bool {
    let s = bone_core(name);
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Tier-2 head/torso selection for converted skeletons with no exact `Head` bone: match
/// the head prefix/digit-tolerantly via `bone_core`, then pick the first ancestor that is
/// neither a (likewise custom-prefixed) neck nor head bone as the spine pivot. Headless
/// rigs (Grafton — top bone `Grafton_BN_C_Spine4`, no head) still return `None`: the 76c
/// oracle's headless pose does not correspond to any bone's frame-0 model transform under
/// the validated sampling method (nearest bone > 40 units), so emitting one would be a
/// guess — stance stays absent and degrades gracefully.
fn select_head_torso_core(skel: &PoseSkeleton) -> Option<(usize, usize)> {
    let head = (0..skel.bone_names.len())
        .find(|&i| core_eq(&skel.bone_names[i], "Head") || core_eq(&skel.bone_names[i], "HeadTwist"))?;
    let mut ancestors: Vec<usize> = Vec::new();
    let mut cur = skel.parent_indices[head];
    while cur >= 0 {
        let a = cur as usize;
        ancestors.push(a);
        cur = skel.parent_indices[a];
    }
    let torso = ancestors
        .iter()
        .copied()
        .find(|&a| {
            let n = &skel.bone_names[a];
            !core_starts_with(n, "Neck") && !core_starts_with(n, "Head")
        })
        .or_else(|| ancestors.first().copied())?;
    Some((head, torso))
}

/// Build the count=1 stance body from an explicit head/torso bone pair (the validation
/// entry — pins the bones against a known oracle). Returns `None` if a bone is missing or
/// the skeleton/clip cannot be sampled.
pub fn emit_stance_count1(
    skeleton_file: &Path,
    clip_file: &Path,
    head_name: &str,
    torso_name: &str,
) -> Option<Vec<u8>> {
    let (skd, pose_skel) = load_skeleton(skeleton_file)?;
    let head = find_bone(&pose_skel.bone_names, head_name)?;
    let torso = find_bone(&pose_skel.bone_names, torso_name)?;
    let (head_m, torso_m) = sample_models(&skd, &pose_skel, clip_file, head, torso)?;
    Some(animation_stance_data_count1_body(
        slot_of(&head_m),
        slot_of(&torso_m),
    ))
}

/// Build the stance body for a creature, auto-selecting the head + spine pivot (the
/// dispatcher entry). When `head_tracking` (the core behavior declares head-tracking),
/// emits the 174 B converted-creature form with a section-2 look-at reference; otherwise
/// the 124 B vanilla form. Returns `None` if the skeleton has no `Head` bone or the idle
/// clip cannot be sampled (caller then writes no file — stance degrades gracefully when
/// absent, so never ship a wrong/empty one).
pub fn emit_stance_for_creature(
    skeleton_file: &Path,
    idle_clip_file: &Path,
    head_tracking: bool,
) -> Option<Vec<u8>> {
    let (skd, pose_skel) = load_skeleton(skeleton_file)?;
    let (head, torso) = select_head_torso(&pose_skel)?;
    let (head_m, torso_m) = sample_models(&skd, &pose_skel, idle_clip_file, head, torso)?;
    let (slot0, slot1) = (slot_of(&head_m), slot_of(&torso_m));
    Some(if head_tracking {
        animation_stance_data_headtrack_body(slot0, slot1, headtrack_lookat(&head_m))
    } else {
        animation_stance_data_count1_body(slot0, slot1)
    })
}

const STANCE_IDENTITY: StanceTransform = ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);

struct ResolvedDonor {
    id: u64,
    sapt: Vec<String>,
    sources: HashSet<PoseRoleSourceKey>,
    data: AnimationStanceData,
}

struct CatalogDonor {
    metadata: WeaponSubgraphMetadata,
    resolved: ResolvedDonor,
}

/// Reusable production context. Construct this once per emission run so base
/// behavior provenance and donor files are decoded once for the whole weapon set.
pub struct WeaponStanceBuilder {
    skel: SkeletonRecord,
    pose_skel: PoseSkeleton,
    resolver: GraphResolver,
    donors: Vec<CatalogDonor>,
}

enum DonorProfile {
    Grid {
        records: Vec<StanceSec2Record>,
        provenance: StanceGridProvenance,
    },
    Trivial,
}

fn family_overlaps(left: &WeaponRaceFamily, right: &WeaponRaceFamily) -> bool {
    [
        &left.owner_race,
        left.sadd.as_ref().unwrap_or(&left.owner_race),
    ]
    .into_iter()
    .any(|left_key| {
        [
            &right.owner_race,
            right.sadd.as_ref().unwrap_or(&right.owner_race),
        ]
        .into_iter()
        .any(|right_key| left_key == right_key)
    })
}

fn ordered_stkd_context_matches(left: &[StanceFormKey], right: &[StanceFormKey]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(target, donor)| {
            !target.plugin.eq_ignore_ascii_case("Fallout4.esm") || target == donor
        })
}

fn metadata_can_supply_donor(
    target: &WeaponSubgraphMetadata,
    donor: &WeaponSubgraphMetadata,
) -> bool {
    target.id != donor.id
        && target.perspective == donor.perspective
        && target.sraf == donor.sraf
        && target
            .core_behavior
            .eq_ignore_ascii_case(&donor.core_behavior)
        && target.sakd == donor.sakd
        && ordered_stkd_context_matches(&target.stkd, &donor.stkd)
        && family_overlaps(&target.race_family, &donor.race_family)
}

fn metadata_owns_donor_context(
    target: &WeaponSubgraphMetadata,
    donor: &WeaponSubgraphMetadata,
) -> bool {
    target.id != donor.id
        && target.perspective == donor.perspective
        && target.sraf == donor.sraf
        && target
            .core_behavior
            .eq_ignore_ascii_case(&donor.core_behavior)
        && target.sakd.len() == donor.sakd.len()
        && target.sakd.first() == donor.sakd.first()
        && ordered_stkd_context_matches(&target.stkd, &donor.stkd)
        && family_overlaps(&target.race_family, &donor.race_family)
}

fn metadata_owns_relaxed_role_context(
    target: &WeaponSubgraphMetadata,
    donor: &WeaponSubgraphMetadata,
) -> bool {
    target.id != donor.id
        && target.perspective == donor.perspective
        && target.sraf == donor.sraf
        && target
            .core_behavior
            .eq_ignore_ascii_case(&donor.core_behavior)
        && target.sakd == donor.sakd
        && family_overlaps(&target.race_family, &donor.race_family)
}

fn owns_power_armor_animation_branch(metadata: &WeaponSubgraphMetadata) -> bool {
    metadata.sapt.first().is_some_and(|branch| {
        branch
            .replace('/', "\\")
            .to_ascii_lowercase()
            .starts_with("actors\\powerarmor\\")
    })
}

fn channels_for_pose(pose_idx: u8, perspective: StancePerspective) -> &'static [u8] {
    const THREE: &[u8] = &[0, 1, 2];
    const TWO: &[u8] = &[0, 1];
    match perspective {
        StancePerspective::FirstPerson => THREE,
        StancePerspective::ThirdPerson if pose_idx < 2 => THREE,
        StancePerspective::ThirdPerson => TWO,
    }
}

fn same_role_source_ignoring_sapt_index(
    left: &PoseRoleSourceKey,
    right: &PoseRoleSourceKey,
) -> bool {
    left.pose_idx == right.pose_idx
        && left.variant == right.variant
        && left.behavior == right.behavior
        && left.branch_ordinal == right.branch_ordinal
        && left.clip_generator == right.clip_generator
        && left.animation_leaf == right.animation_leaf
        && left.sapt_branch == right.sapt_branch
}

fn sapt_overlap(target: &[String], donor: &[String]) -> usize {
    let donor: HashSet<_> = donor
        .iter()
        .map(|branch| branch.replace('/', "\\").to_ascii_lowercase())
        .collect();
    target
        .iter()
        .map(|branch| branch.replace('/', "\\").to_ascii_lowercase())
        .filter(|branch| donor.contains(branch))
        .count()
}

fn donor_profile(
    roles: &[PoseRoleProvenance],
    target_sapt: &[String],
    donors: &[&ResolvedDonor],
    context_donors: &[&ResolvedDonor],
    relaxed_context_donors: &[&ResolvedDonor],
    perspective: StancePerspective,
    centers: &HashMap<(u8, u8), [StanceTransform; 3]>,
    traversal_failure: Option<ProceduralFallbackReason>,
) -> DonorProfile {
    let by_role: HashMap<(u8, u8), &PoseRoleProvenance> = roles
        .iter()
        .map(|role| ((role.pose_idx, role.variant), role))
        .collect();
    let target_sources: HashSet<PoseRoleSourceKey> = roles
        .iter()
        .filter_map(PoseRoleProvenance::source_key)
        .collect();
    let pose_count = match perspective {
        StancePerspective::FirstPerson => 2,
        StancePerspective::ThirdPerson => 6,
    };
    let mut records = Vec::new();
    let mut donor_records = Vec::new();
    let mut procedural_fallbacks = Vec::new();
    let mut procedural_record_count = 0;
    for pose_idx in 0..pose_count {
        for variant in 0..=1 {
            let tags: Vec<u32> = channels_for_pose(pose_idx, perspective)
                .iter()
                .map(|channel| stance_sec2_tag(pose_idx, variant, *channel))
                .collect();
            let role = by_role.get(&(pose_idx, variant)).copied();
            let source = role.and_then(PoseRoleProvenance::source_key);
            let mut matching: Vec<&ResolvedDonor> = source
                .as_ref()
                .map(|source| {
                    donors
                        .iter()
                        .copied()
                        .filter(|donor| donor.sources.contains(source))
                        .collect()
                })
                .unwrap_or_default();
            if matching.is_empty() {
                matching = source
                    .as_ref()
                    .map(|source| {
                        context_donors
                            .iter()
                            .copied()
                            .filter(|donor| donor.sources.contains(source))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            let mut used_relaxed_sapt_index = false;
            if matching.is_empty() {
                matching = source
                    .as_ref()
                    .map(|source| {
                        relaxed_context_donors
                            .iter()
                            .copied()
                            .filter(|donor| {
                                donor.sources.iter().any(|candidate| {
                                    same_role_source_ignoring_sapt_index(source, candidate)
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                used_relaxed_sapt_index = !matching.is_empty();
            }
            matching.sort_by(|left, right| {
                sapt_overlap(target_sapt, &right.sapt)
                    .cmp(&sapt_overlap(target_sapt, &left.sapt))
                    .then_with(|| left.id.cmp(&right.id))
            });
            matching.dedup_by_key(|donor| donor.id);
            let mut provenance_groups: Vec<Vec<&ResolvedDonor>> = Vec::new();
            for donor in matching.iter().copied() {
                if let Some(group) = provenance_groups
                    .iter_mut()
                    .find(|group| group[0].sources == donor.sources)
                {
                    group.push(donor);
                } else {
                    provenance_groups.push(vec![donor]);
                }
            }

            let reason = if let Some(reason) = traversal_failure.clone() {
                Some(reason)
            } else if source.is_none() {
                Some(ProceduralFallbackReason::MissingSourceProvenance { pose_idx, variant })
            } else if matching.is_empty() {
                Some(ProceduralFallbackReason::MissingDonor { pose_idx, variant })
            } else if !used_relaxed_sapt_index
                && (provenance_groups.len() > 1
                    || (provenance_groups[0].len() > 1
                        && provenance_groups[0][0].sources != target_sources))
            {
                Some(ProceduralFallbackReason::AmbiguousDonor {
                    pose_idx,
                    variant,
                    donor_ids: provenance_groups
                        .iter()
                        .flatten()
                        .map(|donor| donor.id)
                        .collect(),
                })
            } else {
                let donor = matching[0];
                let missing: Vec<u32> = tags
                    .iter()
                    .copied()
                    .filter(|tag| donor.data.sec2_by_tag(*tag).is_none())
                    .collect();
                if !missing.is_empty() {
                    Some(ProceduralFallbackReason::MissingDonorTags {
                        pose_idx,
                        variant,
                        donor_id: donor.id,
                        tags: missing,
                    })
                } else if !tags.iter().all(|tag| {
                    donor
                        .data
                        .sec2_by_tag(*tag)
                        .is_some_and(StanceSec2Record::is_grid)
                }) {
                    Some(ProceduralFallbackReason::DonorShape {
                        pose_idx,
                        variant,
                        donor_id: donor.id,
                    })
                } else {
                    for tag in &tags {
                        records.push(donor.data.sec2_by_tag(*tag).unwrap().clone());
                        donor_records.push(StanceDonorSelection {
                            pose_idx,
                            variant,
                            tag: *tag,
                            donor_id: donor.id,
                            source: source.clone().unwrap(),
                        });
                    }
                    None
                }
            };

            if let Some(reason) = reason {
                let stance = centers[&(pose_idx, variant)];
                for (channel, tag) in channels_for_pose(pose_idx, perspective).iter().zip(&tags) {
                    let center = stance[usize::from(*channel)];
                    records.push(StanceSec2Record::grid(
                        *tag,
                        center,
                        procedural_aim_grid(center),
                    ));
                }
                procedural_record_count += tags.len();
                procedural_fallbacks.push(StanceProceduralFallback {
                    pose_idx,
                    variant,
                    tags,
                    reason,
                });
            }
        }
    }

    DonorProfile::Grid {
        records,
        provenance: StanceGridProvenance {
            donor_record_count: donor_records.len(),
            procedural_record_count,
            donor_records,
            procedural_fallbacks,
        },
    }
}

fn find_character_skeleton(mod_root: &Path, base_root: &Path) -> Option<PathBuf> {
    let relative = Path::new("Actors/Character/CharacterAssets/skeleton.hkx");
    [mod_root, base_root]
        .into_iter()
        .map(|root| root.join(relative))
        .find(|path| path.is_file())
}

fn select_weapon_stance_bones(skeleton: &PoseSkeleton) -> Option<(usize, usize, usize)> {
    let (head, upper) = select_head_torso(skeleton)?;
    let lower = usize::try_from(*skeleton.parent_indices.get(upper)?).ok()?;
    Some((head, upper, lower))
}

fn subtract_stance_height(transform: &mut StanceTransform) {
    transform.1[2] -= 40.0;
}

fn sample_weapon_role(
    skel: &SkeletonRecord,
    pose_skel: &PoseSkeleton,
    clip: Option<&Path>,
    bones: (usize, usize, usize),
) -> Result<[StanceTransform; 3], WeaponStanceBuildError> {
    let mut pose = sample_model_pose(skel, pose_skel, clip).ok_or_else(|| {
        WeaponStanceBuildError::PoseClipDecode(
            clip.map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("<reference-pose>")),
        )
    })?;
    let (head, upper, lower) = bones;
    Ok([
        slot_of(&pose.model_at(lower)),
        slot_of(&pose.model_at(upper)),
        slot_of(&pose.model_at(head)),
    ])
}

fn build_weapon_first_section(
    skel: &SkeletonRecord,
    pose_skel: &PoseSkeleton,
    roles: &[PoseRoleProvenance],
    perspective: StancePerspective,
) -> Result<(Vec<StancePose>, HashMap<(u8, u8), [StanceTransform; 3]>), WeaponStanceBuildError> {
    let bones = select_weapon_stance_bones(pose_skel)
        .ok_or(WeaponStanceBuildError::SkeletonBonesMissing)?;
    let pose_count = match perspective {
        StancePerspective::FirstPerson => 2,
        StancePerspective::ThirdPerson => 6,
    };
    let by_role: HashMap<(u8, u8), &PoseRoleProvenance> = roles
        .iter()
        .map(|role| ((role.pose_idx, role.variant), role))
        .collect();
    let mut poses = Vec::with_capacity(pose_count * 2);
    let mut centers = HashMap::with_capacity(pose_count * 2);
    for pose_idx in 0..pose_count as u8 {
        for variant in 0..=1 {
            let derived_crouch = perspective == StancePerspective::FirstPerson && pose_idx == 1;
            let source_pose = if derived_crouch { 0 } else { pose_idx };
            let clip = if variant == 0 {
                None
            } else {
                by_role
                    .get(&(source_pose, variant))
                    .and_then(|role| role.effective_clip.as_deref())
            };
            let mut channel_centers = sample_weapon_role(skel, pose_skel, clip, bones)?;
            if derived_crouch {
                for transform in &mut channel_centers {
                    subtract_stance_height(transform);
                }
            }
            let slots = match perspective {
                StancePerspective::FirstPerson => {
                    [channel_centers[2], channel_centers[1], STANCE_IDENTITY]
                }
                StancePerspective::ThirdPerson => {
                    [channel_centers[2], channel_centers[1], channel_centers[0]]
                }
            };
            poses.push(StancePose {
                pose_idx,
                variant,
                slots,
            });
            centers.insert((pose_idx, variant), channel_centers);
        }
    }
    Ok((poses, centers))
}

fn wxyz_mul(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
    let [lw, lx, ly, lz] = left;
    let [rw, rx, ry, rz] = right;
    [
        lw * rw - lx * rx - ly * ry - lz * rz,
        lw * rx + lx * rw + ly * rz - lz * ry,
        lw * ry - lx * rz + ly * rw + lz * rx,
        lw * rz + lx * ry - ly * rx + lz * rw,
    ]
}

fn rotation_vector_quat(pitch_degrees: f32, yaw_degrees: f32) -> [f32; 4] {
    let pitch = pitch_degrees.to_radians();
    let yaw = yaw_degrees.to_radians();
    let angle = pitch.hypot(yaw);
    if angle == 0.0 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    let scale = (angle * 0.5).sin() / angle;
    [(angle * 0.5).cos(), pitch * scale, 0.0, yaw * scale]
}

fn procedural_aim_grid(center: StanceTransform) -> Vec<StanceTransform> {
    const YAWS: [f32; 7] = [-60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0];
    const PITCHES: [f32; 5] = [-58.0, -30.0, 0.0, 30.0, 58.0];
    let mut cells = Vec::with_capacity(35);
    for yaw in YAWS {
        for pitch in PITCHES {
            cells.push((
                wxyz_mul(center.0, rotation_vector_quat(pitch, yaw)),
                center.1,
            ));
        }
    }
    cells
}

fn generated_records(
    centers: &HashMap<(u8, u8), [StanceTransform; 3]>,
    perspective: StancePerspective,
    grid: bool,
) -> Vec<StanceSec2Record> {
    let pose_count = match perspective {
        StancePerspective::FirstPerson => 2,
        StancePerspective::ThirdPerson => 6,
    };
    let mut records = Vec::new();
    for pose_idx in 0..pose_count {
        for variant in 0..=1 {
            let stance = centers[&(pose_idx, variant)];
            for channel in channels_for_pose(pose_idx, perspective) {
                let center = stance[usize::from(*channel)];
                let tag = stance_sec2_tag(pose_idx, variant, *channel);
                records.push(if grid {
                    StanceSec2Record::grid(tag, center, procedural_aim_grid(center))
                } else {
                    StanceSec2Record::trivial(tag, center)
                });
            }
        }
    }
    records
}

fn load_donor_catalog(
    base_family_subgraphs: &[WeaponSubgraphMetadata],
    resolver: &mut GraphResolver,
    base_stance_data_root: &Path,
) -> Result<Vec<CatalogDonor>, WeaponStanceBuildError> {
    let mut donors = Vec::new();
    for metadata in base_family_subgraphs {
        let path = base_stance_data_root.join(format!("{}.txt", metadata.id));
        if !path.is_file() {
            continue;
        }
        let (perspective, roles) = if metadata.perspective == StancePerspective::FirstPerson
            && metadata.sraf.perspective == 0
            && !owns_power_armor_animation_branch(metadata)
        {
            match resolver.resolve_stance_pose_roles(
                &metadata.core_behavior,
                &metadata.sapt,
                StancePerspective::ThirdPerson,
            ) {
                Ok(roles) => (StancePerspective::ThirdPerson, roles),
                Err(_) => {
                    let Ok(roles) = resolver.resolve_stance_pose_roles(
                        &metadata.core_behavior,
                        &metadata.sapt,
                        StancePerspective::FirstPerson,
                    ) else {
                        continue;
                    };
                    (StancePerspective::FirstPerson, roles)
                }
            }
        } else {
            let Ok(roles) = resolver.resolve_stance_pose_roles(
                &metadata.core_behavior,
                &metadata.sapt,
                metadata.perspective,
            ) else {
                continue;
            };
            (metadata.perspective, roles)
        };
        let sources: HashSet<PoseRoleSourceKey> = roles
            .iter()
            .filter_map(PoseRoleProvenance::source_key)
            .collect();
        if sources.is_empty() {
            continue;
        }
        let body = std::fs::read(&path).map_err(|source| WeaponStanceBuildError::DonorRead {
            id: metadata.id,
            path: path.clone(),
            source,
        })?;
        let data = match decode_animation_stance_data_body(&body) {
            Ok(data) => data,
            Err(StanceDataCodecError::Section2Size(_)) => continue,
            Err(source) => {
                return Err(WeaponStanceBuildError::DonorDecode {
                    id: metadata.id,
                    source,
                });
            }
        };
        let mut normalized_metadata = metadata.clone();
        normalized_metadata.perspective = perspective;
        donors.push(CatalogDonor {
            metadata: normalized_metadata,
            resolved: ResolvedDonor {
                id: metadata.id,
                sapt: metadata.sapt.clone(),
                sources,
                data,
            },
        });
    }
    Ok(donors)
}

impl WeaponStanceBuilder {
    pub fn new(
        base_family_subgraphs: &[WeaponSubgraphMetadata],
        mod_meshes_root: &Path,
        base_meshes_root: &Path,
        base_stance_data_root: &Path,
    ) -> Result<Self, WeaponStanceBuildError> {
        let skeleton_file = find_character_skeleton(mod_meshes_root, base_meshes_root)
            .ok_or(WeaponStanceBuildError::SkeletonMissing)?;
        let (skel, pose_skel) =
            load_skeleton(&skeleton_file).ok_or(WeaponStanceBuildError::SkeletonDecode)?;
        let mut resolver = GraphResolver::new(vec![
            mod_meshes_root.to_path_buf(),
            base_meshes_root.to_path_buf(),
        ]);
        let donors =
            load_donor_catalog(base_family_subgraphs, &mut resolver, base_stance_data_root)?;
        Ok(Self {
            skel,
            pose_skel,
            resolver,
            donors,
        })
    }

    pub fn build(
        &mut self,
        target: &WeaponSubgraphMetadata,
    ) -> Result<WeaponStanceBuild, WeaponStanceBuildError> {
        if target.core_behavior.is_empty() || target.sapt.is_empty() {
            return Err(WeaponStanceBuildError::InvalidMetadata(
                "SGNM and the full SAPT list are required".to_string(),
            ));
        }
        let (perspective, resolved_roles) = if target.perspective == StancePerspective::FirstPerson
            && target.sraf.perspective == 0
            && !owns_power_armor_animation_branch(target)
        {
            match self.resolver.resolve_stance_pose_roles(
                &target.core_behavior,
                &target.sapt,
                StancePerspective::ThirdPerson,
            ) {
                Ok(roles) => (StancePerspective::ThirdPerson, Ok(roles)),
                Err(_) => (
                    StancePerspective::FirstPerson,
                    self.resolver.resolve_stance_pose_roles(
                        &target.core_behavior,
                        &target.sapt,
                        StancePerspective::FirstPerson,
                    ),
                ),
            }
        } else {
            (
                target.perspective,
                self.resolver.resolve_stance_pose_roles(
                    &target.core_behavior,
                    &target.sapt,
                    target.perspective,
                ),
            )
        };
        let traversal_failure = resolved_roles
            .as_ref()
            .err()
            .map(|error| ProceduralFallbackReason::RoleTraversal(error.to_string()));
        let pose_roles = resolved_roles.unwrap_or_default();
        let (poses, centers) =
            build_weapon_first_section(&self.skel, &self.pose_skel, &pose_roles, perspective)?;
        let mut target_context = target.clone();
        target_context.perspective = perspective;
        let mut donors: Vec<&ResolvedDonor> = self
            .donors
            .iter()
            .filter(|donor| metadata_can_supply_donor(&target_context, &donor.metadata))
            .map(|donor| &donor.resolved)
            .collect();
        let context_donors: Vec<&ResolvedDonor> = self
            .donors
            .iter()
            .filter(|donor| metadata_owns_donor_context(&target_context, &donor.metadata))
            .map(|donor| &donor.resolved)
            .collect();
        let relaxed_context_donors: Vec<&ResolvedDonor> = self
            .donors
            .iter()
            .filter(|donor| metadata_owns_relaxed_role_context(&target_context, &donor.metadata))
            .map(|donor| &donor.resolved)
            .collect();
        if donors.is_empty() {
            donors = context_donors.clone();
        }
        let profile =
            if perspective == StancePerspective::FirstPerson && target.sraf.perspective != 0 {
                DonorProfile::Trivial
            } else {
                donor_profile(
                    &pose_roles,
                    &target.sapt,
                    &donors,
                    &context_donors,
                    &relaxed_context_donors,
                    perspective,
                    &centers,
                    traversal_failure,
                )
            };
        let (sec2, grid_source) = match profile {
            DonorProfile::Grid {
                records,
                provenance,
            } => (records, WeaponGridSource::Sidestep(provenance)),
            DonorProfile::Trivial => (
                generated_records(&centers, perspective, false),
                WeaponGridSource::NotApplicable,
            ),
        };
        let body = animation_stance_data_multipose_body(&poses, &sec2)?;
        Ok(WeaponStanceBuild {
            body,
            pose_roles,
            grid_source,
        })
    }
}

/// Build one production weapon `AnimationStanceData` body without consulting an
/// emitted target file. Use [`WeaponStanceBuilder`] when producing multiple files.
pub fn build_weapon_stance_data(
    request: &WeaponStanceRequest<'_>,
    mod_meshes_root: &Path,
    base_meshes_root: &Path,
    base_stance_data_root: &Path,
) -> Result<WeaponStanceBuild, WeaponStanceBuildError> {
    let mut builder = WeaponStanceBuilder::new(
        request.base_family_subgraphs,
        mod_meshes_root,
        base_meshes_root,
        base_stance_data_root,
    )?;
    builder.build(request.target)
}

/// Collect `.hkx` files under `dir`, bounded to `depth` levels (creature anim trees are
/// shallow). Used to locate the representative idle clip.
fn collect_hkx(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_hkx(&p, depth + 1, out);
            } else if p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("hkx"))
            {
                out.push(p);
            }
        }
    }
}

/// Find the creature skeleton `CharacterAssets\skeleton.hkx` under the race dir.
pub fn find_creature_skeleton(race_disk: &Path) -> Option<PathBuf> {
    let p = race_disk.join("CharacterAssets").join("skeleton.hkx");
    p.is_file().then_some(p).or_else(|| {
        // Case/name-tolerant fallback (BA2 extraction may lowercase the dir/file).
        std::fs::read_dir(race_disk.join("CharacterAssets"))
            .ok()?
            .flatten()
            .find_map(|e| {
                let p = e.path();
                (p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.eq_ignore_ascii_case("skeleton.hkx")))
                .then_some(p)
            })
    })
}

/// Find the creature's representative idle/standing clip under `Animations\`. Prefers an
/// `Idle`-named clip, then `stand`/`ambush`. BEST-EFFORT: the precise idle clip per
/// subgraph is the recipe's in-game-gated refinement (`stance_deep.md`: "which clip per
/// subgraph is unconfirmed for multi-clip graphs"); the standing pose is essentially
/// constant across a creature's subgraphs and degrades gracefully when approximate.
/// Idle-clip preference score for a clip stem: `idle`==100, then idle+mt/stand, idle,
/// stand, ambush; 0 = not an idle candidate.
fn idle_score(stem: &str) -> i32 {
    let s = stem.to_ascii_lowercase();
    if s == "idle" {
        100
    } else if s.contains("idle") && (s.contains("mt") || s.contains("stand")) {
        90
    } else if s.contains("idle") {
        80
    } else if s.contains("stand") {
        50
    } else if s.contains("ambush") {
        30
    } else {
        0
    }
}

/// Pick the highest-scoring idle clip from a set of `.hkx` paths (ties: lexically-last path).
fn best_idle_among(anims: Vec<PathBuf>) -> Option<PathBuf> {
    anims
        .into_iter()
        .filter_map(|p| {
            let st = p.file_stem()?.to_str()?.to_string();
            let sc = idle_score(&st);
            (sc > 0).then_some((sc, p))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
        .map(|(_, p)| p)
}

pub fn find_idle_clip(race_disk: &Path) -> Option<PathBuf> {
    let mut anims = Vec::new();
    collect_hkx(&race_disk.join("Animations"), 0, &mut anims);
    best_idle_among(anims)
}

/// Idle clip directly inside `dir` (NON-recursive) — a subgraph's SAPT self-leaf dir is
/// searched in isolation, so the base subgraph's `Animations\` dir picks its own
/// `idle.hkx` and never leaks into a nested `Injured\*\idle.hkx`.
fn best_idle_in_dir(dir: &Path) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("hkx"))
        })
        .collect();
    best_idle_among(files)
}

/// Resolve a subgraph's representative idle clip from its SAPT chain (self-first). Each
/// chain dir `meshes_root.join(entry)` is searched NON-recursively; the first chain entry
/// that yields an idle-scored clip wins. The same rule covers the base subgraph (its
/// `sapt_chain[0]` IS `…\Animations`). (RE: stance_pose_perSubgraph.md.)
pub fn resolve_subgraph_idle(meshes_root: &Path, sapt_chain: &[String]) -> Option<PathBuf> {
    for entry in sapt_chain {
        let rel = entry.trim_end_matches(['\r', '\n']).replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        if let Some(clip) = best_idle_in_dir(&meshes_root.join(rel)) {
            return Some(clip);
        }
    }
    None
}

/// Emit the creature stance body for a race directory (`Actors\<Race>` under meshes),
/// using the creature skeleton + its representative idle clip. The same camera-framing
/// pose is shared by every subgraph of the creature; `head_tracking` selects the 174 B
/// converted-creature form vs the 124 B vanilla form (the caller resolves it from the
/// core behavior). Returns `None` if the skeleton or an idle clip cannot be located
/// (caller writes no file — graceful when absent).
pub fn emit_stance_for_race(race_disk: &Path, head_tracking: bool) -> Option<Vec<u8>> {
    let skel = find_creature_skeleton(race_disk)?;
    let clip = find_idle_clip(race_disk)?;
    emit_stance_for_creature(&skel, &clip, head_tracking)
}

/// Emit the stance body for ONE subgraph, sourcing the frame-0 pose from that subgraph's
/// OWN idle clip (its SAPT self-leaf dir) instead of the shared base idle. Injured-leg
/// subgraphs stand in a distinct limp/crouch, so per-subgraph sourcing moves files 2/3/4
/// from a wrong (base-idle) pose to their correct pose. Falls back to the race-level idle
/// if the chain yields none. `head_tracking` + bone selection are unchanged (skeleton-/
/// core-level, not clip-level). (RE: stance_pose_perSubgraph.md.)
pub fn emit_stance_for_subgraph(
    race_disk: &Path,
    meshes_root: &Path,
    sapt_chain: &[String],
    head_tracking: bool,
) -> Option<Vec<u8>> {
    let skel = find_creature_skeleton(race_disk)?;
    let clip =
        resolve_subgraph_idle(meshes_root, sapt_chain).or_else(|| find_idle_clip(race_disk))?;
    emit_stance_for_creature(&skel, &clip, head_tracking)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim_text_data::bucket_files::StanceSec2Payload;
    use std::path::PathBuf;

    fn extracted_meshes() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4/Meshes")
    }

    fn fo76_skeleton(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../extracted/fo76/meshes/actors")
            .join(rel)
    }

    /// The head-bone selector must recognize converted FO76 skeletons whose head bone
    /// carries a custom namespace/side prefix or embedded digits (`Mothman_BN_C_Head`,
    /// `HB_C_Head`, `Toad_BN_C_Head`, `C_00Head1`, `C_head00`, `jnt_C_head`). Before the
    /// fix `strip_side_prefix` only knew `C_`/`L_`/`R_`, so `select_head_torso` returned
    /// `None` for ~15 head-bearing races and stance was silently absent. The torso pivot
    /// must also skip the (likewise custom-prefixed) neck bones.
    #[test]
    fn selector_covers_custom_prefixed_head_bones() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo76/meshes/actors");
        if !root.is_dir() {
            eprintln!("extracted/fo76 actors absent; skipping");
            return;
        }
        let cases = [
            ("mothman/characterassets/skeleton.hkx", "Mothman_BN_C_Head"),
            ("honeybeast/characterassets/skeleton.hkx", "HB_C_Head"),
            ("radtoad/characterassets/skeleton.hkx", "Toad_BN_C_Head"),
            ("jerseydevil/characterassets/skeleton.hkx", "C_head00"),
            (
                "wendigocolossus/characterassets/skeleton.hkx",
                "jnt_C_head",
            ),
            (
                "atx/redrocketrobot/characterassets/skeleton.hkx",
                "C_00Head1",
            ),
        ];
        let mut checked = 0;
        for (rel, want_head) in cases {
            let p = fo76_skeleton(rel);
            if !p.is_file() {
                continue;
            }
            checked += 1;
            let (_skd, skel) = load_skeleton(&p).expect("skeleton parses");
            let (head, torso) = select_head_torso(&skel)
                .unwrap_or_else(|| panic!("no head/torso selected for {rel}"));
            assert_eq!(skel.bone_names[head], want_head, "head bone for {rel}");
            assert_ne!(head, torso, "torso must differ from head for {rel}");
            let t = skel.bone_names[torso].to_ascii_lowercase();
            assert!(
                !t.contains("neck") && !t.contains("head"),
                "torso pivot for {rel} must be a spine bone, got {}",
                skel.bone_names[torso]
            );
        }
        assert!(
            checked >= 3,
            "expected >=3 fo76 skeleton fixtures, found {checked}"
        );
    }

    /// Headless rigs keep the skip. Grafton's top bone is `Grafton_BN_C_Spine4` (no head),
    /// and the 76c headless oracle pose does not map onto any bone's frame-0 model
    /// transform, so no defensible slot0/slot1 exists — the selector must return `None`
    /// and stance stays absent rather than shipping a guessed body.
    #[test]
    fn headless_grafton_stance_is_skipped() {
        let skel = fo76_skeleton("graftonmonster/characterassets/skeleton.hkx");
        if !skel.is_file() {
            return;
        }
        let (_skd, pose_skel) = load_skeleton(&skel).expect("skeleton parses");
        assert!(
            select_head_torso(&pose_skel).is_none(),
            "headless Grafton has no head bone; stance must stay absent"
        );
    }

    fn read_slot(b: &[u8], off: usize) -> ([f32; 4], [f32; 3]) {
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        (
            [f(off), f(off + 4), f(off + 8), f(off + 12)],
            [f(off + 16), f(off + 20), f(off + 24)],
        )
    }

    fn synthetic_roles(perspective: StancePerspective) -> Vec<PoseRoleProvenance> {
        let pose_count = match perspective {
            StancePerspective::FirstPerson => 2,
            StancePerspective::ThirdPerson => 6,
        };
        let mut roles = Vec::new();
        for pose_idx in 0..pose_count {
            for variant in 0..=1 {
                roles.push(PoseRoleProvenance {
                    pose_idx,
                    variant,
                    behavior: r"Actors\Character\Behaviors\WeaponBehavior.hkx".to_string(),
                    branch_ordinal: pose_idx,
                    branch_name: format!("branch-{pose_idx}"),
                    clip_generator: format!("clip-{pose_idx}"),
                    animation_leaf: format!("idle-{pose_idx}"),
                    effective_clip: None,
                    effective_sapt_index: Some(0),
                    effective_sapt_branch: Some("mod".to_string()),
                    source_clip: None,
                    source_sapt_index: Some(1),
                    source_sapt_branch: Some("base".to_string()),
                });
            }
        }
        roles
    }

    fn grid_donor(id: u64, roles: &[PoseRoleProvenance]) -> ResolvedDonor {
        let mut sec2 = Vec::new();
        for role in roles {
            for channel in channels_for_pose(role.pose_idx, StancePerspective::ThirdPerson) {
                let tag = stance_sec2_tag(role.pose_idx, role.variant, *channel);
                let center = (
                    [1.0, 0.0, 0.0, 0.0],
                    [
                        f32::from(role.pose_idx),
                        f32::from(role.variant),
                        f32::from(*channel),
                    ],
                );
                sec2.push(StanceSec2Record::grid(tag, center, vec![center; 35]));
            }
        }
        ResolvedDonor {
            id,
            sapt: Vec::new(),
            sources: roles
                .iter()
                .filter_map(PoseRoleProvenance::source_key)
                .collect(),
            data: AnimationStanceData {
                poses: Vec::new(),
                sec2,
            },
        }
    }

    fn synthetic_centers(
        perspective: StancePerspective,
    ) -> HashMap<(u8, u8), [StanceTransform; 3]> {
        let pose_count = match perspective {
            StancePerspective::FirstPerson => 2,
            StancePerspective::ThirdPerson => 6,
        };
        let mut centers = HashMap::new();
        for pose_idx in 0..pose_count {
            for variant in 0..=1 {
                centers.insert(
                    (pose_idx, variant),
                    [
                        ([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
                        ([1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
                        ([1.0, 0.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
                    ],
                );
            }
        }
        centers
    }

    #[test]
    fn unique_provenance_selects_all_channel_records_for_each_role() {
        let roles = synthetic_roles(StancePerspective::ThirdPerson);
        let donors = vec![grid_donor(41, &roles)];
        let donor_refs: Vec<_> = donors.iter().collect();
        let DonorProfile::Grid {
            records,
            provenance,
        } = donor_profile(
            &roles,
            &[],
            &donor_refs,
            &[],
            &[],
            StancePerspective::ThirdPerson,
            &synthetic_centers(StancePerspective::ThirdPerson),
            None,
        )
        else {
            panic!("expected SIDESTEP donor profile");
        };
        assert_eq!(records.len(), 28);
        assert_eq!(provenance.donor_record_count, 28);
        assert_eq!(provenance.procedural_record_count, 0);
        assert!(
            provenance
                .donor_records
                .iter()
                .all(|selection| selection.donor_id == 41)
        );
    }

    #[test]
    fn ambiguous_provenance_falls_back_only_affected_role_records() {
        let roles = synthetic_roles(StancePerspective::ThirdPerson);
        let donor = grid_donor(41, &roles);
        let mut conflicting = grid_donor(42, &roles[..1]);
        let conflicting_center = ([0.0, 1.0, 0.0, 0.0], [42.0, 0.0, 0.0]);
        conflicting.data.sec2[0] = StanceSec2Record::grid(
            conflicting.data.sec2[0].tag,
            conflicting_center,
            vec![conflicting_center; 35],
        );
        let mut unrelated = roles[0].source_key().unwrap();
        unrelated.pose_idx = 99;
        conflicting.sources.insert(unrelated);
        let donors = vec![donor, conflicting];
        let donor_refs: Vec<_> = donors.iter().collect();
        let DonorProfile::Grid { provenance, .. } = donor_profile(
            &roles,
            &[],
            &donor_refs,
            &[],
            &[],
            StancePerspective::ThirdPerson,
            &synthetic_centers(StancePerspective::ThirdPerson),
            None,
        ) else {
            panic!("expected mixed grid profile");
        };
        assert_eq!(provenance.donor_record_count, 25);
        assert_eq!(provenance.procedural_record_count, 3);
        assert_eq!(provenance.procedural_fallbacks.len(), 1);
        assert!(matches!(
            provenance.procedural_fallbacks[0].reason,
            ProceduralFallbackReason::AmbiguousDonor { .. }
        ));
    }

    #[test]
    fn equivalent_provenance_carriers_use_stable_lowest_donor_id() {
        let roles = synthetic_roles(StancePerspective::ThirdPerson);
        let donors = vec![grid_donor(42, &roles), grid_donor(41, &roles)];
        let donor_refs: Vec<_> = donors.iter().collect();
        let DonorProfile::Grid { provenance, .. } = donor_profile(
            &roles,
            &[],
            &donor_refs,
            &[],
            &[],
            StancePerspective::ThirdPerson,
            &synthetic_centers(StancePerspective::ThirdPerson),
            None,
        ) else {
            panic!("expected donor grid profile");
        };
        assert_eq!(provenance.donor_record_count, 28);
        assert_eq!(provenance.procedural_record_count, 0);
        assert!(
            provenance
                .donor_records
                .iter()
                .all(|selection| selection.donor_id == 41)
        );
    }

    #[test]
    fn missing_required_tag_falls_back_only_its_joint_channel_records() {
        let roles = synthetic_roles(StancePerspective::ThirdPerson);
        let mut donor = grid_donor(41, &roles);
        donor.data.sec2.pop();
        let DonorProfile::Grid {
            records,
            provenance,
        } = donor_profile(
            &roles,
            &[],
            &[&donor],
            &[],
            &[],
            StancePerspective::ThirdPerson,
            &synthetic_centers(StancePerspective::ThirdPerson),
            None,
        )
        else {
            panic!("expected mixed grid profile");
        };
        assert_eq!(provenance.donor_record_count, 26);
        assert_eq!(provenance.procedural_record_count, 2);
        assert_eq!(provenance.procedural_fallbacks.len(), 1);
        assert!(matches!(
            provenance.procedural_fallbacks[0].reason,
            ProceduralFallbackReason::MissingDonorTags { .. }
        ));
        for record in &records[26..] {
            let StanceSec2Payload::Grid { cells, .. } = &record.payload else {
                panic!("fallback records must be grids");
            };
            assert_eq!(cells.len(), 35);
            assert_eq!(cells[17], record.reference, "neutral cell is the center");
        }
    }

    /// MirelurkKing count=1 stance (124 B): the container framing is BYTE-IDENTICAL to
    /// the CK oracle and the Head/Spine1 model-space frame-0 bones match to the `hkaPose`
    /// accumulation residual (Spine1 shallow ~1e-5, Head deep ~1e-3). This proves the
    /// whole pipeline: skeleton load → clip frame-0 overlay → canonical local→model →
    /// W-first slot serialization → count=1 container. (RE: `stance_work/reemit.py`.)
    #[test]
    fn mirelurkking_count1_stance_byte_exact_container_float_close_bones() {
        let meshes = extracted_meshes();
        let skel = meshes.join("actors/MirelurkKing/characterassets/skeleton.hkx");
        let clip = meshes.join("actors/MirelurkKing/animations/ambushidn/ambush.hkx");
        let oracle_path = meshes.join("AnimTextData/animationstancedata/10084591932766397485.txt");
        if !skel.is_file() || !clip.is_file() || !oracle_path.is_file() {
            eprintln!("extracted/fo4 MirelurkKing absent; skipping");
            return;
        }
        let oracle = std::fs::read(&oracle_path).unwrap();
        let ours = emit_stance_count1(&skel, &clip, "Head", "Spine1").expect("stance emitted");

        assert_eq!(ours.len(), 124, "count=1 stance is 124 bytes");
        assert_eq!(oracle.len(), 124);

        // Container framing byte-identical: header(0..24), version+count(24..32),
        // pose-0 header(32..36), slot2 IDENTITY(92..120), section-2 count(120..124).
        assert_eq!(&ours[0..36], &oracle[0..36], "header/version/count/poseIdx");
        assert_eq!(
            &ours[92..124],
            &oracle[92..124],
            "slot2 IDENTITY + section-2 count"
        );

        // Bones (slot0 Head @36, slot1 Spine1 @64) float-close to the oracle.
        for (label, off, tol) in [("Head", 36usize, 2e-3f32), ("Spine1", 64usize, 1e-3f32)] {
            let (oq, ot) = read_slot(&oracle, off);
            let (q, t) = read_slot(&ours, off);
            for k in 0..4 {
                assert!(
                    (oq[k] - q[k]).abs() < tol,
                    "{label} quat[{k}] {} vs {} (>{tol})",
                    oq[k],
                    q[k]
                );
            }
            for k in 0..3 {
                // Translations are in game units; allow a proportionally larger epsilon.
                assert!(
                    (ot[k] - t[k]).abs() < 0.05,
                    "{label} trans[{k}] {} vs {}",
                    ot[k],
                    t[k]
                );
            }
        }
    }

    /// The auto-selector finds `Head` + a spine/chest ancestor for the MirelurkKing
    /// skeleton (the dispatcher path), and the resulting body is a well-formed 124 B
    /// count=1 container.
    #[test]
    fn auto_select_emits_count1_for_creature() {
        let meshes = extracted_meshes();
        let skel = meshes.join("actors/MirelurkKing/characterassets/skeleton.hkx");
        let clip = meshes.join("actors/MirelurkKing/animations/ambushidn/ambush.hkx");
        if !skel.is_file() || !clip.is_file() {
            eprintln!("extracted/fo4 MirelurkKing absent; skipping");
            return;
        }
        let (_skd, pose_skel) = load_skeleton(&skel).expect("skeleton");
        let (head, torso) = select_head_torso(&pose_skel).expect("head+torso");
        assert!(pose_skel.bone_names[head].eq_ignore_ascii_case("Head"));
        // The pivot must be an ancestor of the head (never the head itself).
        assert_ne!(head, torso);
        let body = emit_stance_for_creature(&skel, &clip, false).expect("stance");
        assert_eq!(body.len(), 124);
        assert_eq!(&body[1..23], b"AnimationBoneTransform");
    }

}
