//! AnimationSpeedInfo contour codec and CK producer topology.
//!
//! Creature contours are generated directly from clip root motion. Weapon contours use the CK
//! intermediate-record model: a typed evaluation recipe whose sampled surfaces and producer
//! metadata come from the behavior evaluator (`evaluate_recipe`) before encoding.
//!
//! The file is a recursive `Collection`/`Individual` contour tree rooted at the creature's
//! **locomotion state machine**, mirroring the behavior graph's generator sub-tree:
//! * SM children are ordered by **`stateId`** (NOT `states[]` array index);
//! * a speed-bound `hkbClipGenerator` (its `variableBindingSet` binds `playbackSpeed`) is an
//!   `Individual`; an SM/blender containing ≥1 such clip is a `Collection`;
//! * no-speed branches are pruned and unary collections collapsed.
//!
//! Each `Individual` carries `direction = normalize3(D)`, `value = |D|/duration` (D = the loop
//! clip's total root displacement, read from the binary `hkaDefaultAnimatedReferenceFrame`),
//! the bound `param` variable, the state's enter-event `clip` slot, the SM-selector `cond`,
//! and a recursive selector-path `entry`. (Field RE: `speedinfo_generate.md`.)
//!
//! ## Locomotion-SM selection (the `sm_path` seed)
//!
//! The `sm_path` seed is derived by walking
//! the **default-state (`startStateId`) chain** from the behavior graph's `rootGenerator`
//! through generator wrappers (layer/modifier/selector) and SMs, and taking the **deepest SM
//! whose subtree still contains a speed-bound clip**. The chain follows each SM's default state,
//! so a non-default combat sibling is naturally excluded; it stops at the first no-speed SM (a
//! creature's idle SM, e.g. Snallygaster `StandingStateMachine`), leaving its parent — the
//! locomotion SM (`IdleLocomotion_SM`) — as the seed. The seed's contour collapses unary down to
//! the first branching SM (`WalkRunJog_NonStrafing_SM`), the root `Collection`.

use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use havok_native::behavior_eval::{
    AnimationPackfile, BehaviorEvaluator, LoadOptions, RootMotionProjection,
    VariableValue as EvaluatorVariableValue,
};
use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

use super::hkx_cache::{FileMemo, behavior_packfile, path_key};
use super::offsets::extract_baked_reference_frame;

mod contour;
mod producer;
mod tiered;
#[cfg(test)]
mod converted_tests;

pub use contour::{
    CenterMode, CollectionContour, CollectionRootMetadata, Contour, ContourCodecError,
    ContourStats, DirectionCurve, Entry as ContourEntry, EntryLink as ContourEntryLink,
    IndividualContour, RootMetadata, SamplePair, SpeedInfoFile, SpeedInfoRoot, SpeedSampledContour,
    decode_speed_info, encode_speed_info, normalize_entry_links,
};
pub use producer::{
    BehaviorGraphOwner, BehaviorReplay, DirectionalSummaryEvaluation, EvaluationRequest,
    EvaluationRequestId, GeneratorSelector, IndividualEvaluation, NeedsEvaluation, PathAction,
    ProducerClass, ProducerRecord, RecipeContour, RecipeEntry, RecipeRecordHandle,
    RecipeRecordParentage, RecipeStats, RootMetadataEvaluation, RootMetadataRecipe, SampleDomain,
    SpeedInfoProducerError, SpeedInfoRootRecipe, SpeedSampledEvaluation,
};

/// Producer metadata captured by the established single-file creature path. Weapon roots do not
/// use this value; their metadata is an explicit evaluator request.
const CREATURE_PRODUCER_METADATA_BITS: u32 = 0x3E08_888D;

/// Vanilla SuperMutant `MTBehavior.hkb/MTDefault`; the sibling grenade/mine roots differ by only
/// 7-16 ULP because CK samples their otherwise-identical transition at slightly different times.
const SHARED_MT_PRODUCER_METADATA_BITS: u32 = 0x3DDD_DF37;

// ---------------------------------------------------------------------------------------
// Object-model navigation (mirrors the AnimEventInfo resolver's accessors).
// ---------------------------------------------------------------------------------------

fn as_i64(v: &HkxValue) -> Option<i64> {
    match v {
        HkxValue::I8(i) => Some(*i as i64),
        HkxValue::U8(i) => Some(*i as i64),
        HkxValue::I16(i) => Some(*i as i64),
        HkxValue::U16(i) => Some(*i as i64),
        HkxValue::I32(i) => Some(*i as i64),
        HkxValue::U32(i) => Some(*i as i64),
        HkxValue::I64(i) => Some(*i),
        HkxValue::U64(i) => Some(*i as i64),
        _ => None,
    }
}

fn i64_member(obj: &HkxObject, name: &str) -> Option<i64> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| as_i64(&m.value))
}

fn f32_member(obj: &HkxObject, name: &str) -> Option<f32> {
    obj.members
        .iter()
        .find(|member| member.name == name)
        .and_then(|member| match member.value {
            HkxValue::F32(value) | HkxValue::Half(value) => Some(value),
            _ => as_i64(&member.value).map(|value| value as f32),
        })
}

fn i64_in(members: &[HkxMember], name: &str) -> Option<i64> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| as_i64(&m.value))
}

fn string_member(obj: &HkxObject, name: &str) -> Option<String> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::String { value, .. } if !value.is_empty() => Some(value.clone()),
            _ => None,
        })
}

fn array_member<'a>(obj: &'a HkxObject, name: &str) -> Option<&'a Vec<HkxValue>> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::Array(items) => Some(items),
            _ => None,
        })
}

fn ptr_targets(v: &HkxValue) -> Vec<usize> {
    match v {
        HkxValue::Pointer(Some(i)) => vec![*i],
        HkxValue::Array(items) => items
            .iter()
            .filter_map(|it| match it {
                HkxValue::Pointer(Some(i)) => Some(*i),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn ptr_array(obj: &HkxObject, name: &str) -> Vec<usize> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .map(|m| ptr_targets(&m.value))
        .unwrap_or_default()
}

fn first_ptr(obj: &HkxObject, name: &str) -> Option<usize> {
    ptr_array(obj, name).into_iter().next()
}

/// The `hkbBehaviorGraphStringData` `variableNames` + `eventNames` (index-preserving).
fn collect_string_data(objects: &[HkxObject]) -> (Vec<String>, Vec<String>) {
    fn list(obj: &HkxObject, name: &str) -> Vec<String> {
        array_member(obj, name)
            .map(|items| {
                items
                    .iter()
                    .map(|v| match v {
                        HkxValue::String { value, .. } => value.clone(),
                        _ => String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    for o in objects {
        if o.class_name == "hkbBehaviorGraphStringData" {
            return (list(o, "variableNames"), list(o, "eventNames"));
        }
    }
    (Vec::new(), Vec::new())
}

/// `stateId -> object index` for one state machine's `states[]`.
fn sm_states(sm: &HkxObject, objects: &[HkxObject]) -> BTreeMap<i64, usize> {
    let mut m = BTreeMap::new();
    for i in ptr_array(sm, "states") {
        if let Some(st) = objects.get(i) {
            if let Some(sid) = i64_member(st, "stateId") {
                m.insert(sid, i);
            }
        }
    }
    m
}

/// The behavior variable bound to `member_path` on `obj` (via its `variableBindingSet`), e.g.
/// `playbackSpeed` → `WalkForwardSpeedMult`, `startStateId` → `iWalkDirection`.
fn bound_var(
    obj: &HkxObject,
    member_path: &str,
    objects: &[HkxObject],
    var_names: &[String],
) -> Option<String> {
    let vbs = objects.get(first_ptr(obj, "variableBindingSet")?)?;
    for b in array_member(vbs, "bindings")? {
        let members: &[HkxMember] = match b {
            HkxValue::Pointer(Some(i)) => match objects.get(*i) {
                Some(o) => o.members.as_slice(),
                None => continue,
            },
            _ => match b.as_object_members() {
                Some(ms) => ms,
                None => continue,
            },
        };
        let mp = members
            .iter()
            .find(|m| m.name == "memberPath")
            .and_then(|m| match &m.value {
                HkxValue::String { value, .. } => Some(value.as_str()),
                _ => None,
            });
        if mp == Some(member_path) {
            let vi = i64_in(members, "variableIndex")?;
            return var_names.get(vi as usize).cloned();
        }
    }
    None
}

fn speed_var_of_clip(
    clip: &HkxObject,
    objects: &[HkxObject],
    var_names: &[String],
) -> Option<String> {
    bound_var(clip, "playbackSpeed", objects, var_names)
}

fn selector_var_of_sm(
    sm: &HkxObject,
    objects: &[HkxObject],
    var_names: &[String],
) -> Option<String> {
    bound_var(sm, "startStateId", objects, var_names)
}

// ---------------------------------------------------------------------------------------
// Speed detection + locomotion-SM seed selection.
// ---------------------------------------------------------------------------------------

/// True iff `idx`'s generator subtree contains a speed-bound `hkbClipGenerator`.
fn subtree_has_speed(
    idx: usize,
    objects: &[HkxObject],
    var_names: &[String],
    seen: &mut HashSet<usize>,
) -> bool {
    if !seen.insert(idx) {
        return false;
    }
    let Some(o) = objects.get(idx) else {
        return false;
    };
    if o.class_name == "hkbClipGenerator" {
        return speed_var_of_clip(o, objects, var_names).is_some();
    }
    for f in [
        "generator",
        "pDefaultGenerator",
        "pBlenderGenerator",
        "child",
    ] {
        if let Some(g) = first_ptr(o, f) {
            if subtree_has_speed(g, objects, var_names, seen) {
                return true;
            }
        }
    }
    for f in ["generators", "children", "states", "layers"] {
        for g in ptr_array(o, f) {
            let target = match objects.get(g) {
                Some(go)
                    if matches!(
                        go.class_name.as_str(),
                        "hkbStateMachineStateInfo" | "hkbBlenderGeneratorChild" | "hkbLayer"
                    ) =>
                {
                    match first_ptr(go, "generator") {
                        Some(gg) => gg,
                        None => continue,
                    }
                }
                _ => g,
            };
            if subtree_has_speed(target, objects, var_names, seen) {
                return true;
            }
        }
    }
    false
}

/// Collect every object index reachable from `idx` via generator/state edges. Used to exclude
/// the `CameraStateMachine` subtree from weapon SpeedInfo root selection: its sync-idle camera
/// SMs bind `iSyncIdleLocomotion` too, but CK ships no SpeedInfo for them.
fn collect_subtree(idx: usize, objects: &[HkxObject], out: &mut HashSet<usize>) {
    if !out.insert(idx) {
        return;
    }
    let Some(o) = objects.get(idx) else {
        return;
    };
    for f in [
        "generator",
        "pDefaultGenerator",
        "pBlenderGenerator",
        "child",
    ] {
        if let Some(g) = first_ptr(o, f) {
            collect_subtree(g, objects, out);
        }
    }
    for f in ["generators", "children", "states", "layers"] {
        for g in ptr_array(o, f) {
            collect_subtree(g, objects, out);
        }
    }
}

/// Follow generator wrappers (layer/modifier/selector/blender) to the first `hkbStateMachine`.
fn descend_to_sm(idx: usize, objects: &[HkxObject], depth: usize) -> Option<usize> {
    if depth > 40 {
        return None;
    }
    let o = objects.get(idx)?;
    match o.class_name.as_str() {
        "hkbStateMachine" => Some(idx),
        "hkbModifierGenerator" => {
            first_ptr(o, "generator").and_then(|g| descend_to_sm(g, objects, depth + 1))
        }
        "DynamicAnimationTaggingGenerator" => {
            first_ptr(o, "pDefaultGenerator").and_then(|g| descend_to_sm(g, objects, depth + 1))
        }
        "BSCyclicBlendTransitionGenerator" => {
            first_ptr(o, "pBlenderGenerator").and_then(|g| descend_to_sm(g, objects, depth + 1))
        }
        "hkbLayerGenerator" => {
            for l in ptr_array(o, "layers") {
                if let Some(g) = objects.get(l).and_then(|lo| first_ptr(lo, "generator")) {
                    if let Some(r) = descend_to_sm(g, objects, depth + 1) {
                        return Some(r);
                    }
                }
            }
            None
        }
        "hkbBlenderGenerator" => {
            for ch in ptr_array(o, "children") {
                if let Some(g) = objects
                    .get(ch)
                    .filter(|c| c.class_name == "hkbBlenderGeneratorChild")
                    .and_then(|c| first_ptr(c, "generator"))
                {
                    if let Some(r) = descend_to_sm(g, objects, depth + 1) {
                        return Some(r);
                    }
                }
            }
            None
        }
        "hkbManualSelectorGenerator" | "hkbPoseMatchingGenerator" => {
            for g in ptr_array(o, "generators") {
                if let Some(r) = descend_to_sm(g, objects, depth + 1) {
                    return Some(r);
                }
            }
            None
        }
        _ => None,
    }
}

/// The locomotion SM = the deepest SM on the default-state chain whose subtree has speed.
fn locomotion_seed_sm(objects: &[HkxObject], var_names: &[String]) -> Option<usize> {
    locomotion_seed_sm_impl(objects, var_names, false)
}

/// Same walk, but when the default-state chain dead-ends at an SM that still has speed (its
/// default state carries no nested SM — Grafton's `RootBehavior`), continue past the dead-end
/// to the UNIQUE speed-bearing child SM. This recovers the locomotion SM (`IdleLocomotion_SM`)
/// that sits off the default path (RE `grafton-animtext-three-emitter-defects`). Used only as a
/// fallback after the plain default-chain seed fails to yield an encodable contour.
fn locomotion_seed_sm_deep(objects: &[HkxObject], var_names: &[String]) -> Option<usize> {
    locomotion_seed_sm_impl(objects, var_names, true)
}

fn locomotion_seed_sm_impl(
    objects: &[HkxObject],
    var_names: &[String],
    follow_dead_end: bool,
) -> Option<usize> {
    let bg = objects
        .iter()
        .find(|o| o.class_name == "hkbBehaviorGraph")?;
    let mut cur = descend_to_sm(first_ptr(bg, "rootGenerator")?, objects, 0);
    let mut seed = None;
    let mut visited = HashSet::new();
    while let Some(sm_idx) = cur {
        if !visited.insert(sm_idx) {
            break;
        }
        if !subtree_has_speed(sm_idx, objects, var_names, &mut HashSet::new()) {
            break;
        }
        seed = Some(sm_idx);
        let sm = &objects[sm_idx];
        let start = i64_member(sm, "startStateId").unwrap_or(0);
        let states = sm_states(sm, objects);
        let default_next = states
            .get(&start)
            .and_then(|&st| first_ptr(&objects[st], "generator"))
            .and_then(|gen_ptr| descend_to_sm(gen_ptr, objects, 0));
        cur = default_next.or_else(|| {
            if !follow_dead_end {
                return None;
            }
            // The default state carried no nested SM. Continue to the unique child SM whose
            // subtree still has speed — the locomotion SM off the default path. Ambiguity
            // (0 or >1 candidates) leaves the chain stopped, exactly as before.
            let mut speed_children = states
                .values()
                .filter_map(|&st| first_ptr(&objects[st], "generator"))
                .filter_map(|gen_ptr| descend_to_sm(gen_ptr, objects, 0))
                .filter(|&s| {
                    s != sm_idx && subtree_has_speed(s, objects, var_names, &mut HashSet::new())
                });
            let first = speed_children.next()?;
            speed_children.next().is_none().then_some(first)
        });
    }
    seed
}

/// `stateId -> enterEventId` from wildcard and per-state transitions. Wildcards are scanned
/// first so the established creature mapping keeps priority when both sources target a state.
fn sm_enter_eventmap(sm: &HkxObject, objects: &[HkxObject]) -> HashMap<i64, i64> {
    let mut m = HashMap::new();
    let mut scan = |transitions: &HkxObject| {
        if let Some(trans) = array_member(transitions, "transitions") {
            for t in trans {
                let members: &[HkxMember] = match t {
                    HkxValue::Pointer(Some(i)) => match objects.get(*i) {
                        Some(o) => o.members.as_slice(),
                        None => continue,
                    },
                    _ => match t.as_object_members() {
                        Some(ms) => ms,
                        None => continue,
                    },
                };
                if let (Some(ev), Some(ts)) =
                    (i64_in(members, "eventId"), i64_in(members, "toStateId"))
                {
                    m.entry(ts).or_insert(ev);
                }
            }
        }
    };
    if let Some(transitions) = first_ptr(sm, "wildcardTransitions").and_then(|i| objects.get(i)) {
        scan(transitions);
    }
    for state_idx in ptr_array(sm, "states") {
        let Some(state) = objects.get(state_idx) else {
            continue;
        };
        if let Some(transitions) = first_ptr(state, "transitions").and_then(|i| objects.get(i)) {
            scan(transitions);
        }
    }
    m
}

/// Number of an SM's states whose generator subtree holds a speed-bound clip.
fn surviving_count(g: &BehaviorGraph, f: usize, sm_idx: usize) -> usize {
    let mut n = 0;
    for sref in ptr_array(&g.objs(f)[sm_idx], "states") {
        let Some(si) = g.objs(f).get(sref) else {
            continue;
        };
        if si.class_name != "hkbStateMachineStateInfo" {
            continue;
        }
        if let Some(gen_idx) = first_ptr(si, "generator") {
            if subtree_has_speed(gen_idx, g.objs(f), g.vars(f), &mut HashSet::new()) {
                n += 1;
            }
        }
    }
    n
}

// ---------------------------------------------------------------------------------------
// Contour tree.
// ---------------------------------------------------------------------------------------

#[derive(Clone)]
struct Leaf {
    param: String,
    clip: String,
    cond: String,
    /// `(file, idx)` of the speed-bound `hkbClipGenerator` (→ its loop clip's root motion).
    speed_clip: (usize, usize),
    /// `(file, sm_idx, state_idx, enterEventId)` from root contour SM down to this leaf's
    /// state — file-aware so a chain that crosses a behavior reference resolves each link in
    /// its own index space.
    ancestors: Vec<(usize, usize, usize, i64)>,
    /// True when the path from the deepest state to this clip crossed a blender edge. Weapon
    /// contours encode such a leaf's `clip` slot as an empty string; the creature serializer
    /// deliberately ignores this field.
    through_blender: bool,
}

#[derive(Clone)]
enum Node {
    Collection(Vec<Node>),
    Individual(Leaf),
}

/// The per-Individual recursive selector path (the "trailer"); `link` ascends toward the root.
struct Entry {
    state_id: i64,
    link: Option<(String, String, Box<Entry>)>,
}

// ---------------------------------------------------------------------------------------
// Cross-file behavior set. A stance SM's locomotion contour descends through a
// `hkbBehaviorReferenceGenerator` whose `behaviorName` names a separate behavior file
// (`WeaponBehavior` → e.g. a directional-locomotion behavior). Each file has its own
// variableBindingSet/event/`hkbBehaviorGraph.name` index space, so instead of merging graphs,
// every object reference is a `(file, idx)` pair and each file keeps its own string data.
// A single-file graph (one `Behavior`) reproduces the creature path exactly.
// ---------------------------------------------------------------------------------------

/// One parsed behavior file plus its per-file string-index spaces. Shared: the same file
/// is reached by every subgraph of a race (and every weapon subgraph re-walks the whole
/// base-game behavior closure), so parsing and string-data collection are memoized by disk
/// path in [`behavior_core`].
struct BehaviorCore {
    objects: Vec<HkxObject>,
    var_names: Vec<String>,
    event_names: Vec<String>,
    graph_name: String,
    root_generator: Option<usize>,
}

/// One behavior file in a graph: the shared parse plus the relpath THIS graph reached it by.
struct Behavior {
    core: Arc<BehaviorCore>,
    /// Normalized (`\`-sep) relpath this file was reached by — `behaviorName`s are resolved
    /// relative to its parent dir. Empty for a single-file (creature) graph.
    rel: String,
}

static BEHAVIOR_CORES: FileMemo<Option<Arc<BehaviorCore>>> = FileMemo::new();

/// Parse a behavior file into its reusable core, memoized by on-disk path.
fn behavior_core(disk: &Path) -> Option<Arc<BehaviorCore>> {
    BEHAVIOR_CORES.get_or_init(&path_key(disk), || {
        let objects = behavior_packfile(disk)?.objects().to_vec();
        let (var_names, event_names) = collect_string_data(&objects);
        let graph = objects.iter().find(|o| o.class_name == "hkbBehaviorGraph");
        let graph_name = graph
            .and_then(|o| string_member(o, "name"))
            .unwrap_or_default();
        let root_generator = graph.and_then(|o| first_ptr(o, "rootGenerator"));
        Some(Arc::new(BehaviorCore {
            objects,
            var_names,
            event_names,
            graph_name,
            root_generator,
        }))
    })
}

pub(super) fn clear_behavior_memo() {
    BEHAVIOR_CORES.clear();
    CLIP_DIRECTORIES.clear();
}

struct BehaviorGraph<'a> {
    files: Vec<Behavior>,
    by_rel: HashMap<String, usize>, // lowercased `\`-rel → file id
    roots: &'a [&'a Path],
}

impl<'a> BehaviorGraph<'a> {
    fn objs(&self, f: usize) -> &[HkxObject] {
        &self.files[f].core.objects
    }
    fn vars(&self, f: usize) -> &[String] {
        &self.files[f].core.var_names
    }
    fn evs(&self, f: usize) -> &[String] {
        &self.files[f].core.event_names
    }

    /// `behaviorName` (relative to `from`'s parent dir) → loaded file id, if reachable.
    fn resolve(&self, from: usize, behavior_name: &str) -> Option<usize> {
        let rel = join_behavior_rel(&behavior_parent_dir(&self.files[from].rel), behavior_name);
        self.by_rel
            .get(&rel.replace('/', "\\").to_ascii_lowercase())
            .copied()
    }

    /// Single-file graph (creature path): just the core, no reference following.
    fn load_single(core_disk: &Path, roots: &'a [&'a Path]) -> Option<Self> {
        Some(BehaviorGraph {
            files: vec![Behavior {
                core: behavior_core(core_disk)?,
                rel: String::new(),
            }],
            by_rel: HashMap::new(),
            roots,
        })
    }

    /// Multi-file graph: the core plus every behavior transitively referenced via
    /// `hkbBehaviorReferenceGenerator.behaviorName`, BFS, resolved across `roots`.
    fn load_reachable(core_rel: &str, roots: &'a [&'a Path]) -> Self {
        let mut g = BehaviorGraph {
            files: Vec::new(),
            by_rel: HashMap::new(),
            roots,
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        queue.push_back(core_rel.to_string());
        while let Some(rel) = queue.pop_front() {
            let key = rel.replace('/', "\\").to_ascii_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            let Some(disk) = find_behavior_on_disk(&rel, roots) else {
                continue;
            };
            let Some(core) = behavior_core(&disk) else {
                continue;
            };
            let parent = behavior_parent_dir(&rel);
            for o in &core.objects {
                if o.class_name == "hkbBehaviorReferenceGenerator" {
                    if let Some(name) = string_member(o, "behaviorName") {
                        queue.push_back(join_behavior_rel(&parent, &name));
                    }
                }
            }
            let fid = g.files.len();
            g.by_rel.insert(key, fid);
            g.files.push(Behavior { core, rel });
        }
        g
    }

    fn load_reachable_checked(
        core_rel: &str,
        roots: &'a [&'a Path],
    ) -> Result<Self, SpeedInfoProducerError> {
        let mut graph = BehaviorGraph {
            files: Vec::new(),
            by_rel: HashMap::new(),
            roots,
        };
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([core_rel.to_string()]);
        while let Some(rel) = queue.pop_front() {
            let key = rel.replace('/', "\\").to_ascii_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            let disk = find_behavior_on_disk(&rel, roots)
                .ok_or_else(|| SpeedInfoProducerError::BehaviorNotFound(rel.clone()))?;
            let core = behavior_core(&disk)
                .ok_or_else(|| SpeedInfoProducerError::BehaviorDecode(rel.clone()))?;
            let parent = behavior_parent_dir(&rel);
            for object in &core.objects {
                if object.class_name == "hkbBehaviorReferenceGenerator" {
                    let behavior_name = string_member(object, "behaviorName").ok_or(
                        SpeedInfoProducerError::MissingProducerData {
                            behavior_file: graph.files.len(),
                            object_index: 0,
                            field: "behaviorName",
                        },
                    )?;
                    queue.push_back(join_behavior_rel(&parent, &behavior_name));
                }
            }
            let file_id = graph.files.len();
            graph.by_rel.insert(key, file_id);
            graph.files.push(Behavior { core, rel });
        }
        Ok(graph)
    }
}

fn build(
    g: &BehaviorGraph,
    f: usize,
    idx: usize,
    ancestors: Vec<(usize, usize, usize, i64)>,
    ref_depth: usize,
    through_blender: bool,
) -> Option<Node> {
    let o = g.objs(f).get(idx)?;
    match o.class_name.as_str() {
        "hkbClipGenerator" => {
            let param = speed_var_of_clip(o, g.objs(f), g.vars(f))?;
            let &(af, sm_idx, _state_idx, enter_ev) = ancestors.last()?;
            let clip = if enter_ev >= 0 {
                g.evs(af)
                    .get(enter_ev as usize)
                    .cloned()
                    .unwrap_or_default()
            } else {
                string_member(o, "name").unwrap_or_default()
            };
            let cond =
                selector_var_of_sm(&g.objs(af)[sm_idx], g.objs(af), g.vars(af)).unwrap_or_default();
            Some(Node::Individual(Leaf {
                param,
                clip,
                cond,
                speed_clip: (f, idx),
                ancestors,
                through_blender,
            }))
        }
        "hkbStateMachine" => {
            let emap = sm_enter_eventmap(o, g.objs(f));
            let mut kids: Vec<(i64, Node)> = Vec::new();
            for sref in ptr_array(o, "states") {
                let Some(si) = g.objs(f).get(sref) else {
                    continue;
                };
                if si.class_name != "hkbStateMachineStateInfo" {
                    continue;
                }
                let stid = i64_member(si, "stateId").unwrap_or(0);
                let Some(gen_idx) = first_ptr(si, "generator") else {
                    continue;
                };
                let mut anc = ancestors.clone();
                anc.push((f, idx, sref, emap.get(&stid).copied().unwrap_or(-1)));
                if let Some(sub) = build(g, f, gen_idx, anc, ref_depth, false) {
                    kids.push((stid, sub));
                }
            }
            if kids.is_empty() {
                return None;
            }
            kids.sort_by_key(|(s, _)| *s);
            let ch: Vec<Node> = kids.into_iter().map(|(_, n)| n).collect();
            Some(collapse(ch))
        }
        "hkbBehaviorReferenceGenerator" => {
            // Cross-file descent: continue the contour in the referenced behavior's own index
            // space, entering at its graph `rootGenerator`. Ancestors carry their own file ids,
            // so the upper (referring-file) links stay resolvable.
            if ref_depth >= 16 {
                return None; // cyclic / pathologically deep behaviorName chain
            }
            let name = string_member(o, "behaviorName")?;
            let tf = g.resolve(f, &name)?;
            let entry = g.files[tf].core.root_generator?;
            build(g, tf, entry, ancestors, ref_depth + 1, through_blender)
        }
        _ => {
            // Same generator-edge set as subtree_has_speed (the selection predicate): a stance
            // descends IN-FILE RifleRelaxed_SM → … → BSCyclicBlendTransitionGenerator
            // --pBlenderGenerator--> directional blend → loop clip. Without pBlenderGenerator/
            // pDefaultGenerator/layers, build() stalls before the locomotion leaves.
            for field in [
                "generators",
                "children",
                "generator",
                "child",
                "pBlenderGenerator",
                "pDefaultGenerator",
                "layers",
            ] {
                let subs: Vec<Node> = ptr_array(o, field)
                    .into_iter()
                    .filter_map(|r| {
                        let crossed_blender = through_blender
                            || matches!(
                                field,
                                "children" | "pBlenderGenerator" | "pDefaultGenerator" | "layers"
                            );
                        build(g, f, r, ancestors.clone(), ref_depth, crossed_blender)
                    })
                    .collect();
                if !subs.is_empty() {
                    return Some(collapse(subs));
                }
            }
            None
        }
    }
}

/// A single surviving child collapses (unary Collection); ≥2 form a Collection.
fn collapse(mut ch: Vec<Node>) -> Node {
    if ch.len() == 1 {
        ch.pop().unwrap()
    } else {
        Node::Collection(ch)
    }
}

fn make_entry(g: &BehaviorGraph, leaf: &Leaf) -> Entry {
    // Entry-chain ceiling = the contour root (first branching SM); unary-collapsed ancestors
    // (e.g. IdleLocomotion_SM, surviving==1) are dropped.
    let anc: Vec<(usize, usize, usize, i64)> = leaf
        .ancestors
        .iter()
        .copied()
        .filter(|a| surviving_count(g, a.0, a.1) > 1)
        .collect();

    if anc.is_empty() {
        return Entry {
            state_id: 0,
            link: None,
        };
    }

    // One trailer per enclosing collection, innermost first, each carrying its own level's enter
    // event, selector and state id: the flat form of
    //   E[dir] link(moveBackward/iSyncDirection) E[dir] link(walkStart/iLocomotionSpeedState) E[speed]
    // that CK writes. Built in full here; `normalize_entry_links` trims it to the depth the tree
    // allows.
    let level = |index: usize| -> (String, String, i64) {
        let (file, state_machine, state, enter_event) = anc[index];
        let event = if enter_event >= 0 {
            g.evs(file)
                .get(enter_event as usize)
                .cloned()
                .unwrap_or_default()
        } else {
            String::new()
        };
        let selector = selector_var_of_sm(&g.objs(file)[state_machine], g.objs(file), g.vars(file))
            .unwrap_or_default();
        let state_id = i64_member(&g.objs(file)[state], "stateId").unwrap_or(0);
        (event, selector, state_id)
    };

    // `anc[0]` is outermost, `anc[len - 1]` innermost.
    let innermost = anc.len() - 1;
    let mut entry = Entry {
        state_id: level(0).2,
        link: None,
    };
    for index in 1..anc.len() {
        let outer = level(index - 1);
        entry = Entry {
            state_id: level(index).2,
            link: Some((outer.0, outer.1, Box::new(entry))),
        };
    }
    let own = level(innermost);
    Entry {
        state_id: own.2,
        link: Some((own.0, own.1, Box::new(entry))),
    }
}

// ---------------------------------------------------------------------------------------
// Root motion (value + direction) from the loop clip's binary reference frame.
// ---------------------------------------------------------------------------------------

fn leaf_basename(animation_name: &str) -> String {
    let norm = animation_name.replace('/', "\\");
    let last = norm.rsplit('\\').next().unwrap_or(&norm);
    match last.rfind('.') {
        Some(d) => &last[..d],
        None => last,
    }
    .to_ascii_lowercase()
}

fn animation_key(animation_name: &str) -> String {
    let normalized = animation_name.replace('/', "\\").to_ascii_lowercase();
    normalized
        .strip_suffix(".hkt")
        .or_else(|| normalized.strip_suffix(".hkx"))
        .unwrap_or(&normalized)
        .to_string()
}

fn resolve_case_insensitive(root: &Path, relative: &Path) -> Option<PathBuf> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let component = component.as_os_str();
        let direct = current.join(component);
        if direct.exists() {
            current = direct;
            continue;
        }
        let entry = std::fs::read_dir(&current).ok()?.flatten().find(|entry| {
            entry
                .file_name()
                .to_str()
                .zip(component.to_str())
                .is_some_and(|(actual, requested)| actual.eq_ignore_ascii_case(requested))
        })?;
        current = entry.path();
    }
    current.exists().then_some(current)
}

fn declared_animation_path(animation_name: &str, sapt: &str) -> Option<PathBuf> {
    let animation_components = animation_name
        .replace('/', "\\")
        .split('\\')
        .map(str::to_string)
        .collect::<Vec<_>>();
    let animation_root = animation_components
        .iter()
        .position(|component| component.eq_ignore_ascii_case("Animations"))?;
    let mut relative = PathBuf::new();
    for component in sapt
        .trim_end_matches(['\r', '\n', ' '])
        .replace('/', "\\")
        .split('\\')
    {
        relative.push(component);
        if component.eq_ignore_ascii_case("Animations") {
            break;
        }
    }
    if !relative
        .components()
        .any(|component| component.as_os_str().eq_ignore_ascii_case("Animations"))
    {
        return None;
    }
    for component in &animation_components[animation_root + 1..] {
        relative.push(component);
    }
    relative.set_extension("hkx");
    Some(relative)
}

static CLIP_DIRECTORIES: FileMemo<Arc<HashMap<String, PathBuf>>> = FileMemo::new();

/// The `.hkx` whose stem equals `leaf`, directly inside `dir` under `root` (non-recursive,
/// case-insensitive). This is the SAPT-override match: the clip resolved at the SAPT dir
/// itself rather than re-anchored at the base `Animations` root.
fn find_clip_stem_in_dir(root: &Path, dir: &str, leaf: &str) -> Option<PathBuf> {
    let clips = CLIP_DIRECTORIES.get_or_init(&path_key(&root.join(dir)), || {
        let mut clips = HashMap::new();
        if let Some(disk_dir) = resolve_case_insensitive(root, Path::new(dir))
            && let Ok(entries) = std::fs::read_dir(disk_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("hkx"))
                    && path.is_file()
                    && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
                {
                    clips.entry(stem.to_ascii_lowercase()).or_insert(path);
                }
            }
        }
        Arc::new(clips)
    });
    clips.get(&leaf.to_ascii_lowercase()).cloned()
}

/// Resolve a speed clip's `animationName` to the on-disk `.hkx` along the subgraph SAPT
/// chain. `roots` is searched in order per SAPT dir (mod first, then base for weapons);
/// the SAPT chain is the override-priority outer loop. Creature callers pass `&[mod]`.
///
/// The clip's basename is looked up in every SAPT dir first (nearest wins), before the
/// declared-path rebuild. `declared_animation_path` re-anchors the name at the base
/// `Animations` root, which would shadow an injured/override clip (the injured MegaSloth
/// RunForward is a limp-run ~3x slower than base).
fn clip_loop_path(clip: &HkxObject, roots: &[&Path], sapt_chain: &[String]) -> Option<PathBuf> {
    let animation_name = string_member(clip, "animationName")?;
    let leaf = leaf_basename(&animation_name);
    // Phase 1: walk the whole SAPT chain, nearest entry first. This is the search path, and
    // it is the only part of the lookup that varies per weapon.
    for sapt in sapt_chain {
        let dir = sapt.trim_end_matches(['\r', '\n', ' ']).replace('\\', "/");
        for root in roots {
            if let Some(p) = find_clip_stem_in_dir(root, &dir, &leaf) {
                return Some(p);
            }
        }
    }
    // Phase 2, only once the chain is exhausted: the graph's own declared path, re-rooted onto
    // the SAPT's prefix-up-to-`Animations`.
    //
    // Must not be interleaved into phase 1. `declared_animation_path` ignores everything in the
    // SAPT after `Animations`, so on an `Actors\Character\Animations\...` chain every entry yields
    // the same path; it would resolve on entry 1 before any grip folder is reached, giving rifle
    // and pistol grips one shared contour. The loop stays because the prefix differs across
    // chains rooted at another actor (`Actors\Supermutant\Animations\...`).
    for sapt in sapt_chain {
        for root in roots {
            if let Some(relative) = declared_animation_path(&animation_name, sapt)
                && let Some(path) = resolve_case_insensitive(root, &relative)
                && path.is_file()
            {
                return Some(path);
            }
        }
    }
    None
}

/// `(value, direction)` from a loop clip: `value = |D|/duration`, `dir = normalize3(D)`, with
/// `D` = the binary reference frame's final displacement (true f32 — NOT the `%f` XML, whose
/// 1-ULP-low duration pushes the value off).
fn value_dir(loop_clip: &Path) -> Option<(f32, [f32; 3])> {
    let rf = extract_baked_reference_frame(loop_clip)?;
    let last = rf.samples.last()?;
    let (dx, dy, dz) = (last[0], last[1], last[2]);
    let mag = (dx * dx + dy * dy + dz * dz).sqrt();
    if rf.duration == 0.0 {
        return None;
    }
    let value = mag / rf.duration;
    let inv = if mag != 0.0 { 1.0 / mag } else { 0.0 };
    Some((value, [dx * inv, dy * inv, dz * inv]))
}

fn creature_contour(node: &Node, g: &BehaviorGraph, sapt_chain: &[String]) -> Option<Contour> {
    match node {
        Node::Collection(ch) => {
            let children: Option<Vec<Contour>> = ch
                .iter()
                .map(|child| creature_contour(child, g, sapt_chain))
                .collect();
            Some(Contour::Collection(CollectionContour {
                children: children?,
            }))
        }
        Node::Individual(leaf) => {
            let (cf, ci) = leaf.speed_clip;
            let loop_clip = clip_loop_path(&g.objs(cf)[ci], g.roots, sapt_chain)?;
            let (speed, direction) = value_dir(&loop_clip)?;
            Some(Contour::Individual(IndividualContour {
                direction,
                parameter: leaf.param.clone(),
                speed,
                clip: leaf.clip.clone(),
                condition: leaf.cond.clone(),
                entry: contour_entry(&make_entry(g, leaf)),
            }))
        }
    }
}

struct ResolvedLeaf {
    source: Leaf,
    dir: [f32; 3],
}

enum ResolvedNode {
    Collection(Vec<ResolvedNode>),
    Individual(ResolvedLeaf),
}

/// Resolve weapon root motion leaf-by-leaf. A missing variant removes only that leaf; empty
/// collections disappear, and a root is rejected only when no leaves remain.
fn resolve_weapon_node(
    node: &Node,
    g: &BehaviorGraph,
    sapt_chain: &[String],
) -> Option<ResolvedNode> {
    match node {
        Node::Collection(children) => {
            let mut resolved: Vec<ResolvedNode> = children
                .iter()
                .filter_map(|child| resolve_weapon_node(child, g, sapt_chain))
                .collect();
            match resolved.len() {
                0 => None,
                1 => resolved.pop(),
                _ => Some(ResolvedNode::Collection(resolved)),
            }
        }
        Node::Individual(source) => {
            let (file, clip_idx) = source.speed_clip;
            let loop_clip = clip_loop_path(&g.objs(file)[clip_idx], g.roots, sapt_chain)?;
            let (_, dir) = value_dir(&loop_clip)?;
            Some(ResolvedNode::Individual(ResolvedLeaf {
                source: source.clone(),
                dir,
            }))
        }
    }
}

fn resolve_mt_locomotion(
    node: &Node,
    g: &BehaviorGraph,
    sapt_chain: &[String],
) -> Option<ResolvedNode> {
    fn collect<'node>(node: &'node Node, leaves: &mut Vec<&'node Leaf>) {
        match node {
            Node::Collection(children) => {
                for child in children {
                    collect(child, leaves);
                }
            }
            Node::Individual(leaf) => leaves.push(leaf),
        }
    }

    let mut leaves = Vec::new();
    collect(node, &mut leaves);
    let children: Vec<ResolvedNode> = ["WalkSpeedMult", "JogSpeedMult", "RunSpeedMult"]
        .into_iter()
        .filter_map(|parameter| {
            leaves
                .iter()
                .filter(|leaf| leaf.param.eq_ignore_ascii_case(parameter))
                .find_map(|leaf| {
                    resolve_weapon_node(&Node::Individual((*leaf).clone()), g, sapt_chain)
                })
        })
        .collect();
    (children.len() > 1).then_some(ResolvedNode::Collection(children))
}

struct DirectionalParent {
    state_id: i64,
    enter_event: String,
    selector: String,
}

struct DirectionalContext {
    key: (usize, usize, i64),
    state_id: i64,
    start_state_id: i64,
    enter_event: String,
    selector: String,
    parent: Option<DirectionalParent>,
    world_center_angle: f32,
}

/// The deepest directional state and its enclosing walk/run state for a blender child.
fn directional_context(g: &BehaviorGraph, leaf: &Leaf) -> Option<DirectionalContext> {
    if !leaf.through_blender {
        return None;
    }
    // `rposition` takes the INNERMOST match, so a direction state machine always wins over the
    // speed-state one outside it. `iLocomotionSpeedState` is here as the fallback for collections
    // whose directions come from a blender with no direction state machine of their own — the
    // walk collection is the case that matters, and vanilla keys it exactly this way
    // (`walkStart|iLocomotionSpeedState`). Without a resolved context a collection keeps source
    // order and gets no chained entry, and since the engine bins every child against the FIRST
    // collection's order, one unresolved collection misaligns all of them.
    let dir_pos = leaf.ancestors.iter().rposition(|(f, sm_idx, _, _)| {
        matches!(
            selector_var_of_sm(&g.objs(*f)[*sm_idx], g.objs(*f), g.vars(*f)).as_deref(),
            Some("iSyncDirection" | "iSyncRunDirection" | "iLocomotionSpeedState")
        )
    })?;
    let (file, sm_idx, state_idx, enter_event_id) = leaf.ancestors[dir_pos];
    let state_id = i64_member(&g.objs(file)[state_idx], "stateId")?;
    let start_state_id = i64_member(&g.objs(file)[sm_idx], "startStateId").unwrap_or(0);
    let enter_event = g
        .evs(file)
        .get(usize::try_from(enter_event_id).ok()?)?
        .clone();
    if enter_event.is_empty() {
        return None;
    }
    let selector = selector_var_of_sm(&g.objs(file)[sm_idx], g.objs(file), g.vars(file))?;
    let states: Vec<i64> = sm_states(&g.objs(file)[sm_idx], g.objs(file))
        .into_keys()
        .collect();
    let ordinal = states.iter().position(|candidate| *candidate == state_id)?;
    let world_center_angle =
        std::f32::consts::FRAC_PI_2 - ordinal as f32 * std::f32::consts::TAU / states.len() as f32;

    let parent = leaf.ancestors[..dir_pos].iter().rev().find_map(
        |(pf, psm_idx, pstate_idx, penter_event_id)| {
            let pselector = selector_var_of_sm(&g.objs(*pf)[*psm_idx], g.objs(*pf), g.vars(*pf))?;
            if pselector != "iLocomotionSpeedState" {
                return None;
            }
            let pstate_id = i64_member(&g.objs(*pf)[*pstate_idx], "stateId")?;
            let penter_event = g
                .evs(*pf)
                .get(usize::try_from(*penter_event_id).ok()?)?
                .clone();
            if penter_event.is_empty() {
                return None;
            }
            Some(DirectionalParent {
                state_id: pstate_id,
                enter_event: penter_event,
                selector: pselector,
            })
        },
    );

    Some(DirectionalContext {
        key: (file, sm_idx, state_id),
        state_id,
        start_state_id,
        enter_event,
        selector,
        parent,
        world_center_angle,
    })
}

fn directional_entry(context: &DirectionalContext, is_last: bool) -> Option<Entry> {
    let terminal = Entry {
        state_id: context.state_id,
        link: None,
    };
    if !is_last {
        return Some(terminal);
    }
    let parent = if context.state_id == context.start_state_id {
        terminal
    } else {
        let parent = context.parent.as_ref()?;
        Entry {
            state_id: context.state_id,
            link: Some((
                parent.enter_event.clone(),
                parent.selector.clone(),
                Box::new(Entry {
                    state_id: parent.state_id,
                    link: None,
                }),
            )),
        }
    };
    Some(Entry {
        state_id: context.state_id,
        link: Some((
            context.enter_event.clone(),
            context.selector.clone(),
            Box::new(parent),
        )),
    })
}

/// Unwrap heading around the forward/backward arc's center before descending-angle order. This
/// keeps the transition-seam endpoint last on both arcs (e.g. +135 degrees is -225 on the back arc).
fn directional_angle(leaf: &ResolvedLeaf, context: &DirectionalContext) -> f32 {
    let center = context.world_center_angle;
    let mut angle = leaf.dir[1].atan2(leaf.dir[0]);
    while angle < center - std::f32::consts::PI {
        angle += std::f32::consts::TAU;
    }
    while angle >= center + std::f32::consts::PI {
        angle -= std::f32::consts::TAU;
    }
    angle
}

/// Encode one subgraph's `AnimationSpeedInfo` body from an explicit locomotion-SM `seed`, or
/// `None` if the seed has no name, the contour is not a `Collection`, or a loop clip's root
/// motion is missing. The encode error itself is intentionally swallowed here (`.ok()`); the
/// caller decides whether an alternate seed can supply a valid contour before surfacing it.
fn speed_info_body_for_seed(
    g: &BehaviorGraph,
    seed: usize,
    graph_name: &str,
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    let sm_name = string_member(&g.objs(0)[seed], "name")?;
    let sm_path = format!("{graph_name}/{sm_name}");
    let tree = build(g, 0, seed, Vec::new(), 0, false)?;
    let contour = creature_contour(&tree, g, sapt_chain)?;
    if !matches!(contour, Contour::Collection(_)) {
        return None;
    }
    encode_speed_info(&SpeedInfoFile {
        roots: vec![SpeedInfoRoot {
            state_machine_path: sm_path,
            contour,
            metadata: RootMetadata::Collection(CollectionRootMetadata {
                center_mode: CenterMode::PiCentered,
                producer: ContourEntry {
                    state_id: -1,
                    value: f32::from_bits(CREATURE_PRODUCER_METADATA_BITS),
                    link: None,
                },
            }),
        }],
    })
    .ok()
}

/// Build one subgraph's `AnimationSpeedInfo` body, or `None` for a non-locomotion creature or
/// when a loop clip's root motion is missing (emit no file rather than a wrong one).
///
/// The primary seed is the default-state-chain locomotion SM. When that chain dead-ends at a
/// container SM whose contour is invalid (Grafton: `RootBehavior` → `InvalidEntryLink`), the
/// deep seed is tried: the walk continued past the dead-end to the unique speed-bearing child
/// SM (`IdleLocomotion_SM`). A seed that never yields an encodable contour is reported as a
/// warning.
pub fn build_speed_info_body(
    core_behavior_disk: &Path,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    let g = BehaviorGraph::load_single(core_behavior_disk, roots)?;
    let graph_name = g.files[0].core.graph_name.clone();
    if graph_name.is_empty() {
        return None;
    }
    let primary = locomotion_seed_sm(g.objs(0), g.vars(0));
    let deep = locomotion_seed_sm_deep(g.objs(0), g.vars(0));
    for seed in primary
        .into_iter()
        .chain(deep.filter(|&d| Some(d) != primary))
    {
        if let Some(body) = speed_info_body_for_seed(&g, seed, &graph_name, sapt_chain) {
            return Some(body);
        }
    }
    // A locomotion SM existed but no seed produced an encodable contour. Warn; the file stays
    // absent so the engine falls back to the Offsets locomotion loops. (Genuine non-locomotion
    // creatures have no seed and stay silent.)
    if primary.is_some() || deep.is_some() {
        eprintln!(
            "AnimationSpeedInfo: no encodable locomotion contour for {} ({graph_name}); \
             emitting no SpeedInfo (locomotion loops remain in Offsets)",
            core_behavior_disk.display()
        );
    }
    None
}

// ---------------------------------------------------------------------------------------
// Weapon / character path: the locomotion contour lives in a base-game behavior referenced by
// the (also base-game) core wrapping behavior, with clips in `extracted/fo4/Meshes`, so the
// single-file, mod-only `build_speed_info_body` can't reach it. Merging object spaces would mix
// per-file variableBindingSet/event index spaces and the wrong `hkbBehaviorGraph.name`, so the
// single-file contour runs on each reachable behavior with its own string data and graph name
// (`WeaponBehavior.hkb`), resolving loop clips across `[mod, base]`.
// ---------------------------------------------------------------------------------------

/// Drop `Behaviors\X.hkx` → the `Actors\<Race>` dir a `behaviorName` is relative to.
fn behavior_parent_dir(rel: &str) -> String {
    let norm = rel.replace('/', "\\");
    let parts: Vec<&str> = norm.split('\\').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        parts[..parts.len() - 2].join("\\")
    } else {
        String::new()
    }
}

fn join_behavior_rel(parent: &str, name: &str) -> String {
    let name = name.replace('/', "\\");
    if parent.is_empty() {
        name
    } else {
        format!("{parent}\\{name}")
    }
}

fn find_behavior_on_disk(rel: &str, roots: &[&Path]) -> Option<PathBuf> {
    for root in roots {
        let p = root.join(rel.replace('\\', "/"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn locomotion_only(node: &Node) -> Option<Node> {
    match node {
        Node::Collection(children) => {
            let mut filtered: Vec<Node> = children.iter().filter_map(locomotion_only).collect();
            match filtered.len() {
                0 => None,
                1 => filtered.pop(),
                _ => Some(Node::Collection(filtered)),
            }
        }
        Node::Individual(leaf) => {
            let parameter = leaf.param.to_ascii_lowercase();
            (parameter.starts_with("flocomotion") && parameter.ends_with("playbackspeed"))
                .then(|| Node::Individual(leaf.clone()))
        }
    }
}

fn generator_children(object: &HkxObject) -> Vec<usize> {
    let mut children = Vec::new();
    for field in [
        "generator",
        "pDefaultGenerator",
        "pBlenderGenerator",
        "child",
        "generators",
        "children",
        "states",
        "layers",
    ] {
        children.extend(ptr_array(object, field));
    }
    children
}

struct LocomotionRootCandidate {
    file: usize,
    state_machine: usize,
    state_machine_path: String,
    tree: Node,
    sampled_source: Option<SampledSource>,
}

fn subtree_contains_object(objects: &[HkxObject], root: usize, target: usize) -> bool {
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    while let Some(index) = pending.pop() {
        if index == target {
            return true;
        }
        if seen.insert(index)
            && let Some(object) = objects.get(index)
        {
            pending.extend(generator_children(object));
        }
    }
    false
}

fn is_overlay_shadow_root(graph: &BehaviorGraph, file: usize, state_machine: usize) -> bool {
    for state in sm_states(&graph.objs(file)[state_machine], graph.objs(file)).into_values() {
        let Some(generator) = first_ptr(&graph.objs(file)[state], "generator") else {
            continue;
        };
        let blender = &graph.objs(file)[generator];
        if blender.class_name != "hkbBlenderGenerator" {
            continue;
        }
        let generators: Vec<usize> = ptr_array(blender, "children")
            .into_iter()
            .filter_map(|child| first_ptr(&graph.objs(file)[child], "generator"))
            .collect();
        let locomotion: Vec<usize> = generators
            .iter()
            .copied()
            .filter(|child| {
                subtree_has_speed(
                    *child,
                    graph.objs(file),
                    graph.vars(file),
                    &mut HashSet::new(),
                )
            })
            .collect();
        if let [locomotion] = locomotion.as_slice()
            && graph.objs(file)[*locomotion].class_name != "hkbStateMachine"
            && generators
                .iter()
                .filter(|generator| *generator != locomotion)
                .all(|generator| graph.objs(file)[*generator].class_name == "hkbStateMachine")
        {
            return true;
        }
    }
    false
}

/// Remove only roots shadowed by source state-machine parentage or by a state-machine overlay
/// around a shared locomotion branch. No animation-clip set comparison participates.
fn remove_shadowed_sampled_roots(
    candidates: &mut Vec<LocomotionRootCandidate>,
    graph: &BehaviorGraph,
) {
    let enclosing: HashSet<(usize, usize)> = candidates
        .iter()
        .filter(|candidate| {
            candidate.sampled_source.is_some()
                && candidates.iter().any(|other| {
                    candidate.state_machine != other.state_machine
                        && candidate.file == other.file
                        && other.sampled_source.is_some()
                        && subtree_contains_object(
                            graph.objs(candidate.file),
                            candidate.state_machine,
                            other.state_machine,
                        )
                })
        })
        .map(|candidate| (candidate.file, candidate.state_machine))
        .collect();
    candidates.retain(|candidate| {
        if enclosing.contains(&(candidate.file, candidate.state_machine)) {
            return false;
        }
        candidate.sampled_source.is_none()
            || !is_overlay_shadow_root(graph, candidate.file, candidate.state_machine)
    });
}

#[derive(Clone)]
struct SourceStateSelection {
    file: usize,
    state_machine: usize,
    state: usize,
}

/// State selections are replayed from the outer graph toward the sampled/leaf generator. Explicit
/// state overrides activate the destination directly; the evaluator emits that state's enter
/// notifications when the root is activated.
fn state_replay(g: &BehaviorGraph, states: &[SourceStateSelection]) -> Vec<BehaviorReplay> {
    let mut replay = Vec::<BehaviorReplay>::new();
    for selection in states {
        let state_id = i64_member(&g.objs(selection.file)[selection.state], "stateId")
            .unwrap_or_default() as i32;
        let owner = BehaviorGraphOwner {
            behavior_file: selection.file,
            relative_path: g.files[selection.file].rel.clone(),
            graph_name: g.files[selection.file].core.graph_name.clone(),
        };
        if replay.last().is_none_or(|segment| segment.owner != owner) {
            replay.push(BehaviorReplay {
                owner,
                root: GeneratorSelector::ObjectIndex(selection.state_machine),
                actions: Vec::new(),
            });
        }
        let actions = &mut replay.last_mut().unwrap().actions;
        if let Some(variable) = selector_var_of_sm(
            &g.objs(selection.file)[selection.state_machine],
            g.objs(selection.file),
            g.vars(selection.file),
        ) {
            actions.push(PathAction::SetVariable {
                name: variable,
                value: EvaluatorVariableValue::Int(state_id),
            });
        }
        actions.push(PathAction::SetState {
            state_machine: GeneratorSelector::ObjectIndex(selection.state_machine),
            state_id,
        });
    }
    replay
}

fn leaf_state_selections(leaf: &Leaf) -> Vec<SourceStateSelection> {
    leaf.ancestors
        .iter()
        .map(|(file, state_machine, state, _)| SourceStateSelection {
            file: *file,
            state_machine: *state_machine,
            state: *state,
        })
        .collect()
}

fn contour_entry(entry: &Entry) -> ContourEntry {
    ContourEntry {
        state_id: entry.state_id as i32,
        value: 0.0,
        link: entry
            .link
            .as_ref()
            .map(|(event, variable, next)| ContourEntryLink {
                event: event.clone(),
                variable: variable.clone(),
                next: Box::new(contour_entry(next)),
            }),
    }
}

struct RecipeBuilder<'graph, 'roots> {
    graph: &'graph BehaviorGraph<'roots>,
    sapt_chain: &'graph [String],
    records: Vec<ProducerRecord>,
    requests: Vec<EvaluationRequest>,
}

impl<'graph, 'roots> RecipeBuilder<'graph, 'roots> {
    fn record(
        &mut self,
        class: ProducerClass,
        parentage: RecipeRecordParentage,
        source_file: usize,
        source_object: usize,
    ) -> Result<RecipeRecordHandle, SpeedInfoProducerError> {
        let handle = u32::try_from(self.records.len())
            .map(RecipeRecordHandle)
            .map_err(|_| SpeedInfoProducerError::RecordHandleOverflow)?;
        let recipe_ordinal = handle.0;
        self.records.push(ProducerRecord {
            handle,
            recipe_ordinal,
            class,
            parentage,
            source_file,
            source_object,
        });
        Ok(handle)
    }

    fn request(
        &mut self,
        request: EvaluationRequest,
    ) -> Result<EvaluationRequestId, SpeedInfoProducerError> {
        let id = u32::try_from(self.requests.len())
            .map(EvaluationRequestId)
            .map_err(|_| SpeedInfoProducerError::RequestIdOverflow)?;
        self.requests.push(request);
        Ok(id)
    }

    fn individual(
        &mut self,
        leaf: &ResolvedLeaf,
        parent: RecipeRecordHandle,
        entry: ContourEntry,
    ) -> Result<RecipeContour, SpeedInfoProducerError> {
        let (file, clip_index) = leaf.source.speed_clip;
        let clip = &self.graph.objs(file)[clip_index];
        let animation_name = string_member(clip, "animationName").ok_or(
            SpeedInfoProducerError::MissingProducerData {
                behavior_file: file,
                object_index: clip_index,
                field: "animationName",
            },
        )?;
        let animation_path = clip_loop_path(clip, self.graph.roots, self.sapt_chain)
            .ok_or_else(|| SpeedInfoProducerError::BehaviorNotFound(animation_name.clone()))?;
        let record = self.record(
            ProducerClass::Individual,
            RecipeRecordParentage::Child { parent },
            file,
            clip_index,
        )?;
        let replay = state_replay(self.graph, &leaf_state_selections(&leaf.source));
        let evaluation = self.request(EvaluationRequest::Individual(IndividualEvaluation {
            record,
            animation_name,
            animation_path,
            playback_parameter: Some(leaf.source.param.clone()),
            replay,
        }))?;
        Ok(RecipeContour::Individual {
            record,
            parameter: leaf.source.param.clone(),
            clip: if leaf.source.through_blender {
                String::new()
            } else {
                leaf.source.clip.clone()
            },
            condition: leaf.source.cond.clone(),
            entry: RecipeEntry::Selector(entry),
            evaluation,
        })
    }

    fn resolved_node(
        &mut self,
        node: &ResolvedNode,
        parentage: RecipeRecordParentage,
        source_file: usize,
        source_object: usize,
    ) -> Result<RecipeContour, SpeedInfoProducerError> {
        match node {
            ResolvedNode::Collection(children) => {
                let record = self.record(
                    ProducerClass::Collection,
                    parentage,
                    source_file,
                    source_object,
                )?;
                let directional: Option<(DirectionalContext, Vec<&ResolvedLeaf>)> = (|| {
                    let mut leaves = Vec::with_capacity(children.len());
                    let mut first_context = None;
                    for child in children {
                        let ResolvedNode::Individual(leaf) = child else {
                            return None;
                        };
                        let context = directional_context(self.graph, &leaf.source)?;
                        if first_context
                            .as_ref()
                            .is_some_and(|first: &DirectionalContext| first.key != context.key)
                        {
                            return None;
                        }
                        first_context.get_or_insert(context);
                        leaves.push(leaf);
                    }
                    let context = first_context?;
                    directional_entry(&context, true)?;
                    Some((context, leaves))
                })(
                );
                let mut recipe_children = Vec::with_capacity(children.len());
                if let Some((context, mut leaves)) = directional {
                    leaves.sort_by(|a, b| {
                        directional_angle(b, &context).total_cmp(&directional_angle(a, &context))
                    });
                    // Vanilla almost never puts two children of one collection on the same angle
                    // (0.76% of 344575 collections, all lean triplets). Converted ones can: the MT
                    // alias table points both diagonals of a pair at one cardinal clip. A repeated
                    // angle leaves the fan unbinnable and the engine falls back to the walk
                    // contour mid-combat. Duplicates share a clip and speed, so keeping the first
                    // loses nothing.
                    let mut kept_angles: Vec<f32> = Vec::with_capacity(leaves.len());
                    leaves.retain(|leaf| {
                        let angle = directional_angle(leaf, &context);
                        if kept_angles.iter().any(|kept| (kept - angle).abs() < 1e-3) {
                            return false;
                        }
                        kept_angles.push(angle);
                        true
                    });
                    let last = leaves.len().saturating_sub(1);
                    for (index, leaf) in leaves.into_iter().enumerate() {
                        let entry = directional_entry(&context, index == last).ok_or(
                            SpeedInfoProducerError::MissingProducerData {
                                behavior_file: source_file,
                                object_index: source_object,
                                field: "directional entry",
                            },
                        )?;
                        recipe_children.push(self.individual(
                            leaf,
                            record,
                            contour_entry(&entry),
                        )?);
                    }
                } else {
                    for child in children {
                        recipe_children.push(self.resolved_node(
                            child,
                            RecipeRecordParentage::Child { parent: record },
                            source_file,
                            source_object,
                        )?);
                    }
                }
                Ok(RecipeContour::Collection {
                    record,
                    children: recipe_children,
                })
            }
            ResolvedNode::Individual(leaf) => {
                let RecipeRecordParentage::Child { parent } = parentage else {
                    return Err(SpeedInfoProducerError::MissingProducerData {
                        behavior_file: source_file,
                        object_index: source_object,
                        field: "Individual parent record",
                    });
                };
                let entry = directional_context(self.graph, &leaf.source)
                    .and_then(|context| directional_entry(&context, true))
                    .map(|entry| contour_entry(&entry))
                    .unwrap_or_else(|| contour_entry(&make_entry(self.graph, &leaf.source)));
                self.individual(leaf, parent, entry)
            }
        }
    }

    fn root_metadata(
        &mut self,
        record: RecipeRecordHandle,
        replay: Vec<BehaviorReplay>,
    ) -> Result<EvaluationRequestId, SpeedInfoProducerError> {
        self.request(EvaluationRequest::RootMetadata(RootMetadataEvaluation {
            record,
            replay,
        }))
    }
}

fn first_resolved_leaf(node: &ResolvedNode) -> Option<&ResolvedLeaf> {
    match node {
        ResolvedNode::Collection(children) => children.iter().find_map(first_resolved_leaf),
        ResolvedNode::Individual(leaf) => Some(leaf),
    }
}

#[derive(Clone)]
struct SampledDirectionSource {
    state: usize,
    transition_event: String,
    direction_blender: usize,
    direction_variable: String,
    speed_variable: String,
    speed_min: f32,
    speed_max: f32,
    direction_min: f32,
    direction_max: f32,
    center_mode: CenterMode,
    lower_animation_names: HashSet<String>,
    summary_clips: Vec<usize>,
}

#[derive(Clone)]
struct SampledSource {
    file: usize,
    speed_state_machine: usize,
    speed_state: usize,
    direction_state_machine: usize,
    direction_selector: String,
    directions: Vec<SampledDirectionSource>,
}

fn transition_event_for_state(
    g: &BehaviorGraph,
    file: usize,
    state_machine: usize,
    state: usize,
) -> Option<String> {
    let state_id = i64_member(&g.objs(file)[state], "stateId")?;
    let event_id = *sm_enter_eventmap(&g.objs(file)[state_machine], g.objs(file)).get(&state_id)?;
    usize::try_from(event_id)
        .ok()
        .and_then(|index| g.evs(file).get(index))
        .filter(|event| !event.is_empty())
        .cloned()
}

/// Slack allowed when deciding whether a direction fan sits on heading 0 or already wraps the
/// whole circle.
const CENTER_TOLERANCE: f32 = std::f32::consts::PI / 72.0;

/// `hkbBlenderGenerator::FLAG_IS_PARAMETRIC_BLEND_CYCLIC`. FO4's blender flags are renumbered
/// against stock Havok, where this bit means something else.
const BLENDER_FLAG_PARAMETRIC_BLEND_CYCLIC: i64 = 0x20;

fn wrap_signed(angle: f32) -> f32 {
    let wrapped = angle.rem_euclid(std::f32::consts::TAU);
    if wrapped > std::f32::consts::PI {
        wrapped - std::f32::consts::TAU
    } else {
        wrapped
    }
}

fn cyclic_distance(value: f32, center: f32) -> f32 {
    let delta = (value - center).rem_euclid(std::f32::consts::TAU);
    delta.min(std::f32::consts::TAU - delta)
}

fn sampled_direction_source(
    g: &BehaviorGraph,
    file: usize,
    direction_state_machine: usize,
    state: usize,
) -> Option<SampledDirectionSource> {
    let transition_event = transition_event_for_state(g, file, direction_state_machine, state)?;
    let generator = first_ptr(&g.objs(file)[state], "generator")?;
    let mut cyclic = Vec::new();
    collect_objects_of_class(
        g.objs(file),
        generator,
        "BSCyclicBlendTransitionGenerator",
        &mut HashSet::new(),
        &mut cyclic,
    );
    for cyclic_index in cyclic {
        let direction_variable = bound_var(
            &g.objs(file)[cyclic_index],
            "fBlendParameter",
            g.objs(file),
            g.vars(file),
        )?;
        let direction_blender = first_ptr(&g.objs(file)[cyclic_index], "pBlenderGenerator")?;
        let direction_children = ptr_array(&g.objs(file)[direction_blender], "children");
        if direction_children.is_empty() {
            continue;
        }
        let mut speed_variable = None;
        let mut speed_min = f32::INFINITY;
        let mut speed_max = f32::NEG_INFINITY;
        let mut lower_animation_names = HashSet::new();
        let mut summary_clips = Vec::new();
        let mut direction_weights = Vec::new();
        let mut valid = true;
        for direction_child in direction_children {
            let direction_child = g
                .objs(file)
                .get(direction_child)
                .filter(|child| child.class_name == "hkbBlenderGeneratorChild");
            let Some(direction_child) = direction_child else {
                valid = false;
                break;
            };
            let Some(direction_weight) = f32_member(direction_child, "weight") else {
                valid = false;
                break;
            };
            direction_weights.push(direction_weight);
            let speed_blender = first_ptr(direction_child, "generator");
            let Some(speed_blender) = speed_blender
                .filter(|index| g.objs(file)[*index].class_name == "hkbBlenderGenerator")
            else {
                valid = false;
                break;
            };
            let Some(variable) = bound_var(
                &g.objs(file)[speed_blender],
                "blendParameter",
                g.objs(file),
                g.vars(file),
            ) else {
                valid = false;
                break;
            };
            if speed_variable
                .as_ref()
                .is_some_and(|known| known != &variable)
            {
                valid = false;
                break;
            }
            speed_variable.get_or_insert(variable);
            let mut endpoints = Vec::new();
            for speed_child in ptr_array(&g.objs(file)[speed_blender], "children") {
                let Some(child) = g.objs(file).get(speed_child) else {
                    valid = false;
                    break;
                };
                let Some(weight) = f32_member(child, "weight") else {
                    valid = false;
                    break;
                };
                speed_min = speed_min.min(weight);
                speed_max = speed_max.max(weight);
                let generator = first_ptr(child, "generator")?;
                endpoints.push((weight, generator));
                let mut clips = Vec::new();
                collect_clips_in_order(g.objs(file), generator, &mut HashSet::new(), &mut clips);
                summary_clips.extend(clips);
            }
            let Some((_, lower_generator)) = endpoints
                .into_iter()
                .min_by(|left, right| left.0.total_cmp(&right.0))
            else {
                valid = false;
                break;
            };
            let mut clips = Vec::new();
            collect_clips_in_order(
                g.objs(file),
                lower_generator,
                &mut HashSet::new(),
                &mut clips,
            );
            lower_animation_names.extend(clips.into_iter().filter_map(|clip| {
                string_member(&g.objs(file)[clip], "animationName").map(|name| leaf_basename(&name))
            }));
        }
        if !valid || !speed_min.is_finite() || !speed_max.is_finite() {
            continue;
        }
        // The direction fan, verbatim from CK. A child's heading is `weight * TAU`; the fan is
        // wrapped into [-PI, PI] only when some child already sits on heading 0, and a cyclic
        // blender whose remaining gap is no wider than its own average step is treated as
        // covering the whole circle. The centre then falls out of the wrapped range rather than
        // out of the sector's index in its state machine. Not `PI/2 - TAU * weight`: that mirrors
        // the fan, which only `CombatWalk*` (weights symmetric about 0.125) survives; vanilla
        // `RelaxedWalkBackward` is pi-centred over [1.5645, 4.7061].
        let child_count = direction_weights.len();
        let has_zero_child = direction_weights
            .iter()
            .any(|weight| cyclic_distance(weight * std::f32::consts::TAU, 0.0) < CENTER_TOLERANCE);
        let angles = direction_weights.iter().map(|weight| {
            let angle = weight * std::f32::consts::TAU;
            if has_zero_child {
                wrap_signed(angle)
            } else {
                angle
            }
        });
        let (mut direction_min, mut direction_max) = angles
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), angle| {
                (low.min(angle), high.max(angle))
            });
        if !direction_min.is_finite() || !direction_max.is_finite() {
            continue;
        }
        let cyclic_blender = i64_in(&g.objs(file)[direction_blender].members, "flags")
            .is_some_and(|flags| flags & BLENDER_FLAG_PARAMETRIC_BLEND_CYCLIC != 0);
        if cyclic_blender && child_count > 1 {
            let span = (direction_max - direction_min).rem_euclid(std::f32::consts::TAU);
            if std::f32::consts::TAU - span < 1.1 * span / (child_count - 1) as f32 {
                direction_min = -std::f32::consts::PI;
                direction_max = std::f32::consts::PI;
            }
        }
        let center_mode = if direction_max - direction_min
            > std::f32::consts::TAU - CENTER_TOLERANCE
            || has_zero_child
        {
            CenterMode::ZeroCentered
        } else {
            CenterMode::PiCentered
        };
        return Some(SampledDirectionSource {
            state,
            transition_event,
            direction_blender,
            direction_variable,
            speed_variable: speed_variable?,
            speed_min,
            speed_max,
            direction_min,
            direction_max,
            center_mode,
            lower_animation_names,
            summary_clips,
        });
    }
    None
}

fn discover_sampled_source(
    g: &BehaviorGraph,
    file: usize,
    root_state_machine: usize,
) -> Option<SampledSource> {
    let mut speed_state_machines = Vec::new();
    collect_state_machines_across_files(
        g,
        file,
        root_state_machine,
        0,
        &mut HashSet::new(),
        &mut speed_state_machines,
    );
    for (file, speed_state_machine) in speed_state_machines {
        let Some(selector) = selector_var_of_sm(
            &g.objs(file)[speed_state_machine],
            g.objs(file),
            g.vars(file),
        )
        .filter(|selector| selector == "iLocomotionSpeedState") else {
            continue;
        };
        for speed_state in sm_states(&g.objs(file)[speed_state_machine], g.objs(file)).values() {
            // Nested direction state machines FIRST. A weapon speed state nests a direction SM
            // (`iSyncDirection`) whose states are the sectors — normally `moveForward` plus
            // `moveBackward`, the pair CK emits wrapped in their own collection. The
            // single-sector fallback below must not run before this: `sampled_direction_source`
            // returns the FIRST `BSCyclicBlendTransitionGenerator` it reaches, which on a weapon
            // graph is the nested SM's *forward* blender, so taking it early collapses the fan
            // to one bare sector and drops 135°–315° — every strafe-right and rearward angle.
            if let Some(generator) = first_ptr(&g.objs(file)[*speed_state], "generator") {
                let mut nested_state_machines = Vec::new();
                collect_objects_of_class(
                    g.objs(file),
                    generator,
                    "hkbStateMachine",
                    &mut HashSet::new(),
                    &mut nested_state_machines,
                );
                for direction_state_machine in nested_state_machines {
                    let Some(direction_selector) = selector_var_of_sm(
                        &g.objs(file)[direction_state_machine],
                        g.objs(file),
                        g.vars(file),
                    ) else {
                        continue;
                    };
                    let states: Vec<usize> =
                        sm_states(&g.objs(file)[direction_state_machine], g.objs(file))
                            .into_values()
                            .collect();
                    if states.is_empty() {
                        continue;
                    }
                    // An unresolved sector is dropped, not fatal: direction state machines
                    // routinely carry transition states holding a plain clip rather than a
                    // Direction-parametric blender, and CK emits no contour for those.
                    let mut directions: Vec<_> = states
                        .iter()
                        .filter_map(|state| {
                            sampled_direction_source(g, file, direction_state_machine, *state)
                        })
                        .collect();
                    if directions.is_empty() {
                        continue;
                    }
                    directions.sort_by_key(|source| match source.center_mode {
                        CenterMode::PiCentered => 0,
                        CenterMode::ZeroCentered => 1,
                    });
                    return Some(SampledSource {
                        file,
                        speed_state_machine,
                        speed_state: *speed_state,
                        direction_state_machine,
                        direction_selector,
                        directions,
                    });
                }
            }
            // Fallback: a speed state can carry the directional blend directly, with no nested
            // direction SM: `LocomotionRoot`'s `MeleeWalkRunState` hangs `MeleeWalkRunState_CBT`
            // off itself, and that CBT's children are the `Speed`-bound blenders spanning
            // walk→run. (Its siblings' nested `MeleeRun_SM` cyclics blend clip generators, not
            // speed blenders, so the scan above rejects them.) One state, one sector: the blend
            // covers the whole circle on `Direction`, as vanilla emits it (angle [-PI, PI],
            // `cond` = this selector). Also reached with no `generator`, so it is not folded
            // into the `if let` above.
            if let Some(direction) =
                sampled_direction_source(g, file, speed_state_machine, *speed_state)
            {
                return Some(SampledSource {
                    file,
                    speed_state_machine,
                    speed_state: *speed_state,
                    direction_state_machine: speed_state_machine,
                    direction_selector: selector,
                    directions: vec![direction],
                });
            }
        }
    }
    None
}

/// Path of state selections from a root generator to a target object, across behavior references.
///
/// The state path feeding a sampled contour's replay can leave the root's file: the melee
/// locomotion source lives in `Locomotion_8wayBlend.hkx`, reached from `MeleeBehavior.hkx`
/// through a `hkbBehaviorReferenceGenerator`. `state_replay` opens a fresh segment per owning
/// file.
///
/// The `?` on a state's `generator` is deliberate: a state machine with a generator-less state
/// abandons that whole branch there rather than skipping the state.
fn find_state_path_across_files(
    g: &BehaviorGraph,
    file: usize,
    current: usize,
    target: (usize, usize),
    ref_depth: usize,
    seen: &mut HashSet<(usize, usize)>,
) -> Option<Vec<SourceStateSelection>> {
    if (file, current) == target {
        return Some(Vec::new());
    }
    if !seen.insert((file, current)) {
        return None;
    }
    let object = g.objs(file).get(current)?;
    if object.class_name == "hkbStateMachine" {
        for state in sm_states(object, g.objs(file)).into_values() {
            let generator = first_ptr(&g.objs(file)[state], "generator")?;
            if let Some(mut path) =
                find_state_path_across_files(g, file, generator, target, ref_depth, seen)
            {
                path.insert(0, source_state_selection(file, current, state));
                seen.remove(&(file, current));
                return Some(path);
            }
        }
    } else if object.class_name == "hkbBehaviorReferenceGenerator" {
        if ref_depth < 16
            && let Some(referenced) = string_member(object, "behaviorName")
                .and_then(|name| g.resolve(file, &name))
                .filter(|referenced| *referenced != file)
            && let Some(entry) = g.files[referenced].core.root_generator
            && let Some(path) =
                find_state_path_across_files(g, referenced, entry, target, ref_depth + 1, seen)
        {
            seen.remove(&(file, current));
            return Some(path);
        }
    } else {
        for child in generator_children(object) {
            if let Some(path) =
                find_state_path_across_files(g, file, child, target, ref_depth, seen)
            {
                seen.remove(&(file, current));
                return Some(path);
            }
        }
    }
    seen.remove(&(file, current));
    None
}

fn source_state_selection(file: usize, state_machine: usize, state: usize) -> SourceStateSelection {
    SourceStateSelection {
        file,
        state_machine,
        state,
    }
}

fn sampled_replay(
    g: &BehaviorGraph,
    root_file: usize,
    root_state_machine: usize,
    source: &SampledSource,
    direction: &SampledDirectionSource,
) -> Option<Vec<BehaviorReplay>> {
    let mut selections = find_state_path_across_files(
        g,
        root_file,
        root_state_machine,
        (source.file, source.speed_state_machine),
        0,
        &mut HashSet::new(),
    )?;
    selections.push(source_state_selection(
        source.file,
        source.speed_state_machine,
        source.speed_state,
    ));
    selections.push(source_state_selection(
        source.file,
        source.direction_state_machine,
        direction.state,
    ));
    Some(state_replay(g, &selections))
}

fn resolved_animation_names(node: &ResolvedNode, g: &BehaviorGraph, out: &mut HashSet<String>) {
    match node {
        ResolvedNode::Collection(children) => {
            for child in children {
                resolved_animation_names(child, g, out);
            }
        }
        ResolvedNode::Individual(leaf) => {
            let (file, clip) = leaf.source.speed_clip;
            if let Some(name) = string_member(&g.objs(file)[clip], "animationName") {
                out.insert(leaf_basename(&name));
            }
        }
    }
}

fn sampled_insertion_index(
    resolved: &ResolvedNode,
    source: &SampledSource,
    g: &BehaviorGraph,
) -> Option<usize> {
    let ResolvedNode::Collection(children) = resolved else {
        return None;
    };
    let lower_names: HashSet<&str> = source
        .directions
        .iter()
        .flat_map(|direction| direction.lower_animation_names.iter().map(String::as_str))
        .collect();
    children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let mut names = HashSet::new();
            resolved_animation_names(child, g, &mut names);
            let matches = names
                .iter()
                .filter(|name| lower_names.contains(name.as_str()))
                .count();
            (matches, index + 1)
        })
        .filter(|(matches, _)| *matches > 0)
        .max_by_key(|(matches, _)| *matches)
        .map(|(_, insertion)| insertion)
}

fn insert_sampled_recipe(
    builder: &mut RecipeBuilder,
    root_file: usize,
    root_state_machine: usize,
    root: &mut RecipeContour,
    source: &SampledSource,
    insertion: usize,
) -> Result<(), SpeedInfoProducerError> {
    let RecipeContour::Collection {
        record: root_record,
        children,
    } = root
    else {
        return Err(SpeedInfoProducerError::UnsupportedProducerClass {
            class_name: "SpeedSampled root without Collection parent".to_string(),
            behavior_file: source.file,
            object_index: source.speed_state_machine,
        });
    };
    // A lone sampled contour is emitted directly under the root; the engine reads a
    // `collection x1` as a distinct node and stops merging the root's remaining children there.
    let sampled_collection = (source.directions.len() > 1)
        .then(|| {
            builder.record(
                ProducerClass::Collection,
                RecipeRecordParentage::Child {
                    parent: *root_record,
                },
                source.file,
                source.direction_state_machine,
            )
        })
        .transpose()?;
    let sampled_parent = sampled_collection.unwrap_or(*root_record);
    let mut sampled_children = Vec::with_capacity(source.directions.len());
    for direction in &source.directions {
        let record = builder.record(
            ProducerClass::SpeedSampled,
            RecipeRecordParentage::Child {
                parent: sampled_parent,
            },
            source.file,
            direction.direction_blender,
        )?;
        let domain = SampleDomain::historical_fo4(
            direction.direction_variable.clone(),
            direction.speed_variable.clone(),
            direction.direction_min,
            direction.direction_max,
            direction.speed_min,
            direction.speed_max,
        );
        let replay = sampled_replay(
            builder.graph,
            root_file,
            root_state_machine,
            source,
            direction,
        )
        .ok_or(SpeedInfoProducerError::MissingProducerData {
            behavior_file: source.file,
            object_index: source.speed_state_machine,
            field: "sampled source state path",
        })?;
        let directional_summary = direction
            .summary_clips
            .iter()
            .map(|clip_index| {
                let clip = &builder.graph.objs(source.file)[*clip_index];
                let animation_name = string_member(clip, "animationName").ok_or(
                    SpeedInfoProducerError::MissingProducerData {
                        behavior_file: source.file,
                        object_index: *clip_index,
                        field: "animationName",
                    },
                )?;
                let animation_path = clip_loop_path(clip, builder.graph.roots, builder.sapt_chain)
                    .ok_or_else(|| {
                        SpeedInfoProducerError::BehaviorNotFound(animation_name.clone())
                    })?;
                Ok(DirectionalSummaryEvaluation {
                    animation_name,
                    animation_path,
                })
            })
            .collect::<Result<Vec<_>, SpeedInfoProducerError>>()?;
        let evaluation =
            builder.request(EvaluationRequest::SpeedSampled(SpeedSampledEvaluation {
                record,
                domain,
                direction_input_scale: 1.0,
                directional_summary,
                replay,
            }))?;
        sampled_children.push(RecipeContour::SpeedSampled {
            record,
            center_mode: direction.center_mode,
            clip: direction.transition_event.clone(),
            condition: source.direction_selector.clone(),
            entry: RecipeEntry::Selector(ContourEntry {
                state_id: i64_member(&builder.graph.objs(source.file)[direction.state], "stateId")
                    .unwrap_or_default() as i32,
                value: 0.0,
                link: None,
            }),
            evaluation,
        });
    }
    // The speed state's own trailer. Every contour node carries
    // `center_mode, clip, condition, state_id, value`; for a sampled group the trailer names the
    // speed state (`walkRunBlendStart` on `iLocomotionSpeedState`), not the direction sectors.
    // It is the chained link off the last child, because the wrapping collection's `center_mode`
    // byte of 1 is the link marker. A single-sector group has no wrapping collection, so the bare
    // contour carries the same trailer (vanilla: 10,966 bare vs 41,679 paired).
    if let Some(RecipeContour::SpeedSampled {
        entry: RecipeEntry::Selector(entry),
        ..
    }) = sampled_children.last_mut()
    {
        let speed_objects = builder.graph.objs(source.file);
        if let (Some(event), Some(variable)) = (
            transition_event_for_state(
                builder.graph,
                source.file,
                source.speed_state_machine,
                source.speed_state,
            ),
            selector_var_of_sm(
                &speed_objects[source.speed_state_machine],
                speed_objects,
                builder.graph.vars(source.file),
            ),
        ) {
            entry.link = Some(ContourEntryLink {
                event,
                variable,
                next: Box::new(ContourEntry {
                    state_id: i64_member(&speed_objects[source.speed_state], "stateId")
                        .unwrap_or_default() as i32,
                    value: 0.0,
                    link: None,
                }),
            });
        }
    }
    let node = match sampled_collection {
        Some(record) => RecipeContour::Collection {
            record,
            children: sampled_children,
        },
        None => sampled_children
            .pop()
            .ok_or(SpeedInfoProducerError::MissingProducerData {
                behavior_file: source.file,
                object_index: source.direction_state_machine,
                field: "sampled direction",
            })?,
    };
    children.insert(insertion.min(children.len()), node);
    Ok(())
}

fn collect_clips_in_order(
    objects: &[HkxObject],
    object_index: usize,
    seen: &mut HashSet<usize>,
    out: &mut Vec<usize>,
) {
    if !seen.insert(object_index) {
        return;
    }
    let Some(object) = objects.get(object_index) else {
        return;
    };
    if object.class_name == "hkbClipGenerator" {
        out.push(object_index);
        return;
    }
    for child in generator_children(object) {
        collect_clips_in_order(objects, child, seen, out);
    }
}

fn collect_objects_of_class(
    objects: &[HkxObject],
    object_index: usize,
    class_name: &str,
    seen: &mut HashSet<usize>,
    out: &mut Vec<usize>,
) {
    if !seen.insert(object_index) {
        return;
    }
    let Some(object) = objects.get(object_index) else {
        return;
    };
    if object.class_name == class_name {
        out.push(object_index);
    }
    for child in generator_children(object) {
        collect_objects_of_class(objects, child, class_name, seen, out);
    }
}

/// `collect_objects_of_class`, but descending through `hkbBehaviorReferenceGenerator` into the
/// referenced behavior's own index space — the same cross-file descent `build` performs.
///
/// The locomotion selector a sampled contour keys on (`iLocomotionSpeedState`) usually lives in
/// a *referenced* behavior rather than the root's own file: of FO4's 44 Character behaviors, 17
/// declare it, `Locomotion_8wayBlend.hkx` among them. Staying in-file finds it for a graph small
/// enough to hold its own locomotion, and never for `MeleeBehavior`/`MTBehavior`.
fn collect_state_machines_across_files(
    g: &BehaviorGraph,
    file: usize,
    object_index: usize,
    ref_depth: usize,
    seen: &mut HashSet<(usize, usize)>,
    out: &mut Vec<(usize, usize)>,
) {
    if !seen.insert((file, object_index)) {
        return;
    }
    let Some(object) = g.objs(file).get(object_index) else {
        return;
    };
    if object.class_name == "hkbStateMachine" {
        out.push((file, object_index));
    }
    if object.class_name == "hkbBehaviorReferenceGenerator" {
        if ref_depth >= 16 {
            return; // cyclic / pathologically deep behaviorName chain
        }
        let Some(target) = string_member(object, "behaviorName")
            .and_then(|name| g.resolve(file, &name))
            .filter(|target| *target != file)
        else {
            return;
        };
        if let Some(entry) = g.files[target].core.root_generator {
            collect_state_machines_across_files(g, target, entry, ref_depth + 1, seen, out);
        }
        return;
    }
    for child in generator_children(object) {
        collect_state_machines_across_files(g, file, child, ref_depth, seen, out);
    }
}

fn horizontal_clip_path(
    g: &BehaviorGraph,
    file: usize,
    clip_index: usize,
    sapt_chain: &[String],
) -> Option<PathBuf> {
    let path = clip_loop_path(&g.objs(file)[clip_index], g.roots, sapt_chain)?;
    let (_, direction) = value_dir(&path)?;
    ((direction[0] * direction[0] + direction[1] * direction[1]).sqrt() > 0.5).then_some(path)
}

fn direct_cyclic_collection(
    g: &BehaviorGraph,
    file: usize,
    state_machine: usize,
    sapt_chain: &[String],
) -> Option<(i32, Vec<(usize, PathBuf)>)> {
    for (state_id, state) in sm_states(&g.objs(file)[state_machine], g.objs(file)) {
        let generator = first_ptr(&g.objs(file)[state], "generator")?;
        let mut cyclic = Vec::new();
        collect_objects_of_class(
            g.objs(file),
            generator,
            "BSCyclicBlendTransitionGenerator",
            &mut HashSet::new(),
            &mut cyclic,
        );
        for cyclic_index in cyclic {
            if bound_var(
                &g.objs(file)[cyclic_index],
                "fBlendParameter",
                g.objs(file),
                g.vars(file),
            )
            .is_none()
            {
                continue;
            }
            let blender = first_ptr(&g.objs(file)[cyclic_index], "pBlenderGenerator")?;
            let mut resolved = Vec::new();
            let mut seen_clips = HashSet::new();
            for child in ptr_array(&g.objs(file)[blender], "children") {
                let Some(generator) = g
                    .objs(file)
                    .get(child)
                    .filter(|object| object.class_name == "hkbBlenderGeneratorChild")
                    .and_then(|object| first_ptr(object, "generator"))
                else {
                    continue;
                };
                let Some(clip) = linear_clip_descendant(g.objs(file), generator) else {
                    continue;
                };
                if !seen_clips.insert(clip) {
                    continue;
                }
                if let Some(path) = horizontal_clip_path(g, file, clip, sapt_chain) {
                    resolved.push((clip, path));
                }
            }
            if !resolved.is_empty() {
                return Some((state_id as i32, resolved));
            }
        }
    }
    None
}

fn linear_clip_descendant(objects: &[HkxObject], object_index: usize) -> Option<usize> {
    let object = objects.get(object_index)?;
    if object.class_name == "hkbClipGenerator" {
        return Some(object_index);
    }
    let children = generator_children(object);
    match children.as_slice() {
        [child] => linear_clip_descendant(objects, *child),
        _ => None,
    }
}

fn direct_state_leaf(
    g: &BehaviorGraph,
    file: usize,
    state_machine: usize,
    sapt_chain: &[String],
) -> Option<(usize, PathBuf, i32)> {
    let enter_events = sm_enter_eventmap(&g.objs(file)[state_machine], g.objs(file));
    if enter_events.is_empty() {
        return None;
    }
    for (state_id, state_index) in sm_states(&g.objs(file)[state_machine], g.objs(file)) {
        if !enter_events.contains_key(&state_id) {
            continue;
        }
        let generator = first_ptr(&g.objs(file)[state_index], "generator")?;
        let Some(clip) = linear_clip_descendant(g.objs(file), generator) else {
            continue;
        };
        if let Some(path) = horizontal_clip_path(g, file, clip, sapt_chain) {
            return Some((clip, path, state_id as i32));
        }
    }
    None
}

impl<'graph, 'roots> RecipeBuilder<'graph, 'roots> {
    fn direct_individual(
        &mut self,
        file: usize,
        clip_index: usize,
        animation_path: PathBuf,
        parentage: RecipeRecordParentage,
        replay: Vec<BehaviorReplay>,
        entry: RecipeEntry,
    ) -> Result<RecipeContour, SpeedInfoProducerError> {
        let animation_name = string_member(&self.graph.objs(file)[clip_index], "animationName")
            .ok_or(SpeedInfoProducerError::MissingProducerData {
                behavior_file: file,
                object_index: clip_index,
                field: "animationName",
            })?;
        let record = self.record(ProducerClass::Individual, parentage, file, clip_index)?;
        let evaluation = self.request(EvaluationRequest::Individual(IndividualEvaluation {
            record,
            animation_name,
            animation_path,
            playback_parameter: None,
            replay,
        }))?;
        Ok(RecipeContour::Individual {
            record,
            parameter: String::new(),
            clip: String::new(),
            condition: String::new(),
            entry,
            evaluation,
        })
    }
}

fn root_replay(
    g: &BehaviorGraph,
    file: usize,
    state_machine: usize,
    state_id: i32,
) -> Option<Vec<BehaviorReplay>> {
    let state = *sm_states(&g.objs(file)[state_machine], g.objs(file)).get(&(state_id as i64))?;
    Some(state_replay(
        g,
        &[source_state_selection(file, state_machine, state)],
    ))
}

/// Construct source-topology records and evaluation work for one weapon subgraph. Runtime
/// `AnimationSpeedInformation` stores roots in a `BSTHashMap<BSFixedString, ...>`, so file order is
/// not semantic. Recipes use deterministic reachable-file/object order and keep it stable.
/// Returned topology contains no sampled values and cannot be serialized until an independent
/// behavior evaluator reconciles every request.
pub fn build_speed_info_recipe_weapon(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Result<NeedsEvaluation, SpeedInfoProducerError> {
    let g = BehaviorGraph::load_reachable_checked(core_rel, roots)?;
    let mut locomotion_candidates = Vec::new();
    let mut selector_candidates = Vec::new();
    for f in 0..g.files.len() {
        let objects = g.objs(f);
        let graph_name = g.files[f].core.graph_name.clone();
        if graph_name.is_empty() {
            continue;
        }
        let mut camera_subtree: HashSet<usize> = HashSet::new();
        if let Some(cam) = objects.iter().position(|o| {
            o.class_name == "hkbStateMachine"
                && string_member(o, "name").as_deref() == Some("CameraStateMachine")
        }) {
            collect_subtree(cam, objects, &mut camera_subtree);
        }
        for idx in 0..objects.len() {
            let obj = &objects[idx];
            if obj.class_name != "hkbStateMachine" {
                continue;
            }
            if selector_var_of_sm(obj, objects, g.vars(f)).as_deref() != Some("iSyncIdleLocomotion")
            {
                continue;
            }
            if camera_subtree.contains(&idx) {
                continue;
            }
            let Some(sm_name) = string_member(obj, "name") else {
                return Err(SpeedInfoProducerError::MissingProducerData {
                    behavior_file: f,
                    object_index: idx,
                    field: "name",
                });
            };
            let sm_path = format!("{graph_name}/{sm_name}");
            selector_candidates.push((f, idx, sm_path.clone()));
            if let Some(tree) = build(&g, f, idx, Vec::new(), 0, false)
                .as_ref()
                .and_then(locomotion_only)
            {
                let sampled_source = discover_sampled_source(&g, f, idx);
                locomotion_candidates.push(LocomotionRootCandidate {
                    file: f,
                    state_machine: idx,
                    state_machine_path: sm_path,
                    tree,
                    sampled_source,
                });
            }
        }
    }

    remove_shadowed_sampled_roots(&mut locomotion_candidates, &g);
    let mut locomotion_by_object: BTreeMap<(usize, usize), LocomotionRootCandidate> =
        locomotion_candidates
            .into_iter()
            .map(|candidate| ((candidate.file, candidate.state_machine), candidate))
            .collect();
    let locomotion_like: HashSet<(usize, usize)> = selector_candidates
        .iter()
        .filter_map(|(file, state_machine, _)| {
            build(&g, *file, *state_machine, Vec::new(), 0, false)
                .as_ref()
                .and_then(locomotion_only)
                .map(|_| (*file, *state_machine))
        })
        .collect();

    let mut builder = RecipeBuilder {
        graph: &g,
        sapt_chain,
        records: Vec::new(),
        requests: Vec::new(),
    };
    let mut recipe_roots = Vec::new();

    for (file, state_machine, state_machine_path) in selector_candidates {
        if let Some(source) = tiered::source(&g, file, state_machine, sapt_chain) {
            recipe_roots.push(tiered::recipe_root(
                &mut builder, file, state_machine, state_machine_path, source,
            )?);
            continue;
        }
        if let Some(candidate) = locomotion_by_object.remove(&(file, state_machine)) {
            let Some(resolved) = resolve_weapon_node(&candidate.tree, &g, sapt_chain) else {
                continue;
            };
            let first_leaf = first_resolved_leaf(&resolved).ok_or(
                SpeedInfoProducerError::MissingProducerData {
                    behavior_file: file,
                    object_index: state_machine,
                    field: "locomotion leaf",
                },
            )?;
            let metadata_replay = state_replay(&g, &leaf_state_selections(&first_leaf.source));
            let sampled_source = candidate.sampled_source.clone();
            let sampled_insertion = sampled_source
                .as_ref()
                .and_then(|source| sampled_insertion_index(&resolved, source, &g));
            let mut contour = builder.resolved_node(
                &resolved,
                RecipeRecordParentage::SourceRoot {
                    behavior_file: file,
                    state_machine,
                },
                file,
                state_machine,
            )?;
            if let Some(source) = sampled_source {
                let insertion =
                    sampled_insertion.ok_or(SpeedInfoProducerError::MissingProducerData {
                        behavior_file: file,
                        object_index: state_machine,
                        field: "sampled contour placement",
                    })?;
                insert_sampled_recipe(
                    &mut builder,
                    file,
                    state_machine,
                    &mut contour,
                    &source,
                    insertion,
                )?;
            }
            let record = match &contour {
                RecipeContour::Collection { record, .. } => *record,
                _ => {
                    return Err(SpeedInfoProducerError::UnsupportedProducerClass {
                        class_name: "non-Collection locomotion root".to_string(),
                        behavior_file: file,
                        object_index: state_machine,
                    });
                }
            };
            let metadata_evaluation = builder.root_metadata(record, metadata_replay)?;
            recipe_roots.push(SpeedInfoRootRecipe {
                record,
                state_machine_path: candidate.state_machine_path,
                contour,
                metadata: RootMetadataRecipe::Collection {
                    center_mode: CenterMode::PiCentered,
                    evaluation: metadata_evaluation,
                },
            });
            continue;
        }
        if locomotion_like.contains(&(file, state_machine)) {
            continue;
        }

        if let Some((state_id, direct_children)) =
            direct_cyclic_collection(&g, file, state_machine, sapt_chain)
        {
            let record = builder.record(
                ProducerClass::Collection,
                RecipeRecordParentage::SourceRoot {
                    behavior_file: file,
                    state_machine,
                },
                file,
                state_machine,
            )?;
            let replay = root_replay(&g, file, state_machine, state_id).ok_or(
                SpeedInfoProducerError::MissingProducerData {
                    behavior_file: file,
                    object_index: state_machine,
                    field: "direct collection source state",
                },
            )?;
            let mut children = Vec::with_capacity(direct_children.len());
            for (clip_index, path) in direct_children {
                children.push(builder.direct_individual(
                    file,
                    clip_index,
                    path,
                    RecipeRecordParentage::Child { parent: record },
                    replay.clone(),
                    RecipeEntry::Selector(ContourEntry {
                        state_id: -1,
                        value: 0.0,
                        link: None,
                    }),
                )?);
            }
            let metadata_evaluation = builder.root_metadata(record, replay)?;
            recipe_roots.push(SpeedInfoRootRecipe {
                record,
                state_machine_path,
                contour: RecipeContour::Collection { record, children },
                metadata: RootMetadataRecipe::Collection {
                    center_mode: CenterMode::ZeroCentered,
                    evaluation: metadata_evaluation,
                },
            });
            continue;
        }

        if let Some((clip_index, animation_path, state_id)) =
            direct_state_leaf(&g, file, state_machine, sapt_chain)
        {
            let replay = root_replay(&g, file, state_machine, state_id).ok_or(
                SpeedInfoProducerError::MissingProducerData {
                    behavior_file: file,
                    object_index: state_machine,
                    field: "direct Individual source state",
                },
            )?;
            let record = builder.record(
                ProducerClass::Individual,
                RecipeRecordParentage::SourceRoot {
                    behavior_file: file,
                    state_machine,
                },
                file,
                clip_index,
            )?;
            let animation_name = string_member(&g.objs(file)[clip_index], "animationName").ok_or(
                SpeedInfoProducerError::MissingProducerData {
                    behavior_file: file,
                    object_index: clip_index,
                    field: "animationName",
                },
            )?;
            let individual_evaluation =
                builder.request(EvaluationRequest::Individual(IndividualEvaluation {
                    record,
                    animation_name,
                    animation_path,
                    playback_parameter: None,
                    replay: replay.clone(),
                }))?;
            let metadata_evaluation = builder.root_metadata(record, replay)?;
            recipe_roots.push(SpeedInfoRootRecipe {
                record,
                state_machine_path,
                contour: RecipeContour::Individual {
                    record,
                    parameter: String::new(),
                    clip: String::new(),
                    condition: String::new(),
                    entry: RecipeEntry::RootMetadata(metadata_evaluation),
                    evaluation: individual_evaluation,
                },
                metadata: RootMetadataRecipe::DirectIndividual {
                    evaluation: metadata_evaluation,
                },
            });
        }
    }

    if recipe_roots.is_empty() {
        return Err(SpeedInfoProducerError::NoSpeedInfoTopology);
    }
    Ok(NeedsEvaluation {
        records: builder.records,
        roots: recipe_roots,
        requests: builder.requests,
    })
}

fn is_shared_mt_behavior(core_rel: &str) -> bool {
    core_rel
        .replace('/', "\\")
        .eq_ignore_ascii_case(r"Actors\Character\Behaviors\MTBehavior.hkx")
}

/// Construct the role-0 locomotion contours mounted by humanoid creatures. The shared MT graph is
/// not a weapon graph: its ordinary walk/jog/run leaves bind `WalkSpeedMult`/`JogSpeedMult`/
/// `RunSpeedMult`, so the weapon-only `fLocomotion*PlaybackSpeed` filter removes every useful
/// root and can leave only a camera contour. Select the sync-locomotion state machines directly
/// and retain the Collection roots whose clips resolve through this race's SAPT chain.
fn build_speed_info_body_mt(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    fn contour(
        node: &ResolvedNode,
        graph: &BehaviorGraph,
        sapt_chain: &[String],
    ) -> Option<Contour> {
        match node {
            ResolvedNode::Collection(children) => Some(Contour::Collection(CollectionContour {
                children: children
                    .iter()
                    .map(|child| contour(child, graph, sapt_chain))
                    .collect::<Option<Vec<_>>>()?,
            })),
            ResolvedNode::Individual(leaf) => {
                let (file, clip) = leaf.source.speed_clip;
                let animation = clip_loop_path(&graph.objs(file)[clip], graph.roots, sapt_chain)?;
                let (speed, direction) = value_dir(&animation)?;
                Some(Contour::Individual(IndividualContour {
                    direction,
                    parameter: leaf.source.param.clone(),
                    speed,
                    clip: leaf.source.clip.clone(),
                    condition: leaf.source.cond.clone(),
                    entry: contour_entry(&make_entry(graph, &leaf.source)),
                }))
            }
        }
    }

    let g = BehaviorGraph::load_reachable(core_rel, roots);
    let file = 0;
    let graph_name = g
        .files
        .first()
        .map(|behavior| behavior.core.graph_name.clone())
        .filter(|name| !name.is_empty())?;
    let mut speed_roots = Vec::new();

    for state_machine in 0..g.objs(file).len() {
        let object = &g.objs(file)[state_machine];
        if object.class_name != "hkbStateMachine"
            || selector_var_of_sm(object, g.objs(file), g.vars(file)).as_deref()
                != Some("iSyncIdleLocomotion")
        {
            continue;
        }
        let Some(tree) = build(&g, file, state_machine, Vec::new(), 0, false) else {
            continue;
        };
        let Some(resolved) = resolve_mt_locomotion(&tree, &g, sapt_chain) else {
            continue;
        };
        let resolved_contour = contour(&resolved, &g, sapt_chain)?;
        if !matches!(resolved_contour, Contour::Collection(_)) {
            continue;
        }
        let sm_name = string_member(object, "name")?;
        speed_roots.push(SpeedInfoRoot {
            state_machine_path: format!("{graph_name}/{sm_name}"),
            contour: resolved_contour,
            metadata: RootMetadata::Collection(CollectionRootMetadata {
                center_mode: CenterMode::PiCentered,
                producer: ContourEntry {
                    state_id: -1,
                    value: f32::from_bits(SHARED_MT_PRODUCER_METADATA_BITS),
                    link: None,
                },
            }),
        });
    }

    if speed_roots.is_empty() {
        return None;
    }
    encode_speed_info(&SpeedInfoFile { roots: speed_roots }).ok()
}

struct EvaluationAssets {
    behaviors: HashMap<usize, Arc<HkxFile>>,
    animation_names: Vec<String>,
    animations: Vec<HkxFile>,
}

enum EvaluatedRequest {
    Individual {
        speed: f32,
        direction: [f32; 3],
    },
    SpeedSampled {
        curves: Vec<DirectionCurve>,
        directional_summary: Vec<SamplePair>,
    },
    RootMetadata(f32),
}

fn evaluation_assets(
    recipe: &NeedsEvaluation,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Result<EvaluationAssets, SpeedInfoProducerError> {
    let mut owners = BTreeMap::<usize, String>::new();
    let mut animation_paths = BTreeMap::<String, (String, PathBuf)>::new();
    for request in &recipe.requests {
        let replay = match request {
            EvaluationRequest::Individual(request) => {
                animation_paths
                    .entry(animation_key(&request.animation_name))
                    .or_insert_with(|| {
                        (
                            request.animation_name.clone(),
                            request.animation_path.clone(),
                        )
                    });
                &request.replay
            }
            EvaluationRequest::SpeedSampled(request) => {
                for summary in &request.directional_summary {
                    animation_paths
                        .entry(animation_key(&summary.animation_name))
                        .or_insert_with(|| {
                            (
                                summary.animation_name.clone(),
                                summary.animation_path.clone(),
                            )
                        });
                }
                &request.replay
            }
            EvaluationRequest::RootMetadata(request) => &request.replay,
        };
        for segment in replay {
            match owners.entry(segment.owner.behavior_file) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(segment.owner.relative_path.clone());
                }
                std::collections::btree_map::Entry::Occupied(entry)
                    if entry.get() != &segment.owner.relative_path =>
                {
                    return Err(SpeedInfoProducerError::Evaluation(format!(
                        "behavior file {} has conflicting paths {:?} and {:?}",
                        segment.owner.behavior_file,
                        entry.get(),
                        segment.owner.relative_path
                    )));
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
            }
        }
    }

    let mut behaviors = HashMap::new();
    for (file, relative_path) in owners {
        let disk = find_behavior_on_disk(&relative_path, roots)
            .ok_or_else(|| SpeedInfoProducerError::BehaviorNotFound(relative_path.clone()))?;
        let hkx = behavior_packfile(&disk).ok_or_else(|| {
            SpeedInfoProducerError::Evaluation(format!(
                "could not read behavior {}",
                disk.display()
            ))
        })?;
        behaviors.insert(file, hkx);
    }
    for behavior in behaviors.values() {
        for object in behavior
            .objects()
            .iter()
            .filter(|object| object.class_name == "hkbClipGenerator")
        {
            let Some(animation_name) = string_member(object, "animationName") else {
                continue;
            };
            let Some(animation_path) = clip_loop_path(object, roots, sapt_chain) else {
                continue;
            };
            animation_paths
                .entry(animation_key(&animation_name))
                .or_insert((animation_name, animation_path));
        }
    }

    let mut animation_names = Vec::with_capacity(animation_paths.len());
    let mut animations = Vec::with_capacity(animation_paths.len());
    for (_, (name, path)) in animation_paths {
        let bytes = std::fs::read(&path)
            .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
        let hkx = read_packfile(&bytes)
            .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
        animation_names.push(name);
        animations.push(hkx);
    }
    Ok(EvaluationAssets {
        behaviors,
        animation_names,
        animations,
    })
}

fn animation_sources(assets: &EvaluationAssets) -> Vec<AnimationPackfile<'_>> {
    assets
        .animation_names
        .iter()
        .zip(&assets.animations)
        .map(|(name, animation)| AnimationPackfile::new(name, animation))
        .collect()
}

fn replay_options(
    replay: &[BehaviorReplay],
) -> Result<(usize, LoadOptions), SpeedInfoProducerError> {
    let Some(innermost) = replay.last() else {
        return Err(SpeedInfoProducerError::Evaluation(
            "evaluation replay is empty".to_string(),
        ));
    };
    // Referenced behavior graphs cannot be loaded as one evaluator. The innermost replay segment
    // already contains the local root and state selections needed to evaluate that graph directly.
    let local_start = replay
        .iter()
        .rposition(|segment| segment.owner != innermost.owner)
        .map_or(0, |index| index + 1);
    let local = &replay[local_start..];
    let first = &local[0];
    Ok((
        first.owner.behavior_file,
        LoadOptions {
            root: first.root.clone(),
            actions: local
                .iter()
                .flat_map(|segment| segment.actions.iter().cloned())
                .collect(),
            root_motion_projection: RootMotionProjection::MagnitudeOnly,
        },
    ))
}

/// Slack applied to the direction-fan step count so a span that lands within float noise of a
/// whole number of steps takes the extra curve, the way CK does.
const DIRECTION_COUNT_EPSILON: f32 = 1.0e-4;

/// Plain truncation matches 96.15% of the CK-built goldens; the rest is one knife-edge fan,
/// `[-1.5770791, 1.5645132]`, whose span is a hair under 12 steps yet gets 13 curves. With the
/// slack all 6136 sampled nodes across `B21_PlasmaCaster` and the Gauss Pistol / Meltdown fan
/// mods match. That fan is the forward arc; without its last bin, directions past ~74.6 degrees
/// get no curve.
fn direction_curve_count(min: f32, max: f32, step: f32) -> u32 {
    ((max - min) / step + DIRECTION_COUNT_EPSILON).trunc() as u32 + 1
}

fn monotonic_speed_samples(samples: &[SamplePair]) -> Vec<SamplePair> {
    if samples.is_empty() {
        return Vec::new();
    }
    let minimum = samples[..samples.len().min(4)]
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| left.output.total_cmp(&right.output))
        .map(|(index, _)| index)
        .unwrap_or_default();
    let mut reduced = vec![samples[minimum]];
    for sample in &samples[minimum + 1..] {
        if sample.output > reduced.last().unwrap().output {
            reduced.push(*sample);
        }
    }
    reduced
}

fn chord_reduce_speed_samples(samples: &[SamplePair], tolerance: f32) -> Vec<SamplePair> {
    if samples.len() <= 1 {
        // A flat sweep survives `monotonic_speed_samples` as a single point, and CK still writes
        // the closing sample, so the curve reads `[(80, 0), (80, 0)]` rather than one lone point.
        // Vanilla never ships a curve with fewer than two samples in 16011 sampled nodes — a
        // one-sample curve gives the engine nothing to interpolate for that heading.
        return samples.iter().chain(samples.iter()).copied().collect();
    }
    let mut reduced = vec![samples[0]];
    let mut anchor = 0usize;
    for candidate in 2..samples.len() {
        let a = samples[anchor];
        let b = samples[candidate];
        let span = (candidate - anchor) as f32;
        let mut max_error = 0.0_f32;
        for (index, actual) in samples.iter().enumerate().take(candidate).skip(anchor + 1) {
            let t = (index - anchor) as f32 / span;
            let input = a.input + (b.input - a.input) * t;
            let output = a.output + (b.output - a.output) * t;
            let dx = input - actual.input;
            let dy = output - actual.output;
            max_error = max_error.max((dx * dx + dy * dy).sqrt());
        }
        if max_error > tolerance {
            anchor = candidate - 1;
            reduced.push(samples[anchor]);
        }
    }
    if reduced.last() != samples.last() {
        reduced.push(*samples.last().unwrap());
    }
    reduced
}

#[cfg(test)]
mod chord_reduce_tests {
    use super::{SamplePair, chord_reduce_speed_samples};

    /// Vanilla ships no curve with fewer than two samples, so a sweep that collapses to one point
    /// still has to close on itself.
    #[test]
    fn a_flat_sweep_still_closes_on_its_own_sample() {
        let flat = [SamplePair {
            input: 80.0,
            output: 0.0,
        }];
        let reduced = chord_reduce_speed_samples(&flat, 2.0);
        assert_eq!(reduced.len(), 2);
        assert_eq!(reduced[0], reduced[1]);
        assert!(chord_reduce_speed_samples(&[], 2.0).is_empty());
    }
}

fn evaluate_speed_sampled(
    request: &SpeedSampledEvaluation,
    assets: &EvaluationAssets,
) -> Result<EvaluatedRequest, SpeedInfoProducerError> {
    let (behavior_file, options) = replay_options(&request.replay)?;
    let behavior = assets.behaviors.get(&behavior_file).ok_or_else(|| {
        SpeedInfoProducerError::Evaluation(format!("behavior file {behavior_file} was not loaded"))
    })?;
    let sources = animation_sources(assets);
    let mut evaluator = BehaviorEvaluator::load(behavior, &sources, options)
        .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
    let domain = &request.domain;
    let direction_count = direction_curve_count(
        domain.direction_min,
        domain.direction_max,
        domain.direction_step,
    );
    let speed_count =
        ((domain.speed_max - domain.speed_min) / domain.speed_step).trunc() as u32 + 1;
    let mut curves = Vec::with_capacity(direction_count as usize);
    let mut previous_sweep: Vec<SamplePair> = Vec::new();
    let mut direction = domain.direction_min;
    for _ in 0..direction_count {
        evaluator
            .set_variable(
                &domain.direction_variable,
                EvaluatorVariableValue::Real(direction * request.direction_input_scale),
            )
            .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
        evaluator
            .advance_repeated(domain.timestep, domain.warmup_updates)
            .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
        let mut samples = Vec::with_capacity(speed_count as usize);
        let mut requested_speed = domain.speed_min;
        for _ in 0..speed_count {
            evaluator
                .set_variable(
                    &domain.speed_variable,
                    EvaluatorVariableValue::Real(requested_speed),
                )
                .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
            evaluator
                .advance_repeated(domain.timestep, domain.warmup_updates)
                .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
            evaluator.reset_root_translation();
            let mut measured_speed = 0.0_f32;
            for _ in 0..domain.measurement_updates {
                let translation = evaluator
                    .advance(domain.timestep)
                    .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
                let magnitude = (translation.x * translation.x
                    + translation.y * translation.y
                    + translation.z * translation.z)
                    .sqrt();
                measured_speed += magnitude * domain.displacement_scale;
            }
            measured_speed *= 1.0 / domain.measurement_updates as f32;
            samples.push(SamplePair {
                input: requested_speed,
                output: measured_speed,
            });
            evaluator.reset_root_translation();
            requested_speed += domain.speed_step;
        }
        let sweep = monotonic_speed_samples(&samples);
        let reduced = chord_reduce_speed_samples(&sweep, domain.chord_error_tolerance);
        // CK reuses one sample buffer across directions and never clears it, so every curve but
        // the first is written as the *previous* direction's full sweep followed by its own
        // reduced one. Vanilla, the CK-built B21 golden and the CK-built fan mods all show the
        // resulting x-axis reset in curves 1..n and none in curve 0, so the engine is tuned
        // against this shape and parity requires reproducing it.
        let mut samples = std::mem::replace(&mut previous_sweep, sweep);
        samples.extend_from_slice(&reduced);
        curves.push(DirectionCurve {
            angle: direction,
            samples,
        });
        direction += domain.direction_step;
    }

    let directional_summary = request
        .directional_summary
        .iter()
        .map(|summary| {
            let (speed, direction) = value_dir(&summary.animation_path).ok_or_else(|| {
                SpeedInfoProducerError::Evaluation(format!(
                    "could not read root motion from {}",
                    summary.animation_path.display()
                ))
            })?;
            Ok(SamplePair {
                input: direction[1].atan2(direction[0]),
                output: speed,
            })
        })
        .collect::<Result<Vec<_>, SpeedInfoProducerError>>()?;
    Ok(EvaluatedRequest::SpeedSampled {
        curves,
        directional_summary,
    })
}

fn evaluate_root_metadata(
    request: &RootMetadataEvaluation,
    assets: &EvaluationAssets,
) -> Result<EvaluatedRequest, SpeedInfoProducerError> {
    let (behavior_file, options) = replay_options(&request.replay)?;
    let behavior = assets.behaviors.get(&behavior_file).ok_or_else(|| {
        SpeedInfoProducerError::Evaluation(format!("behavior file {behavior_file} was not loaded"))
    })?;
    let sources = animation_sources(assets);
    let mut evaluator = BehaviorEvaluator::load_with_trace(behavior, &sources, options)
        .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
    evaluator
        .advance(0.0)
        .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
    let advance = evaluator.trace().advances.first();
    let metadata = advance
        .into_iter()
        .flat_map(|advance| {
            advance
                .transitions
                .iter()
                .map(|transition| transition.duration)
                .chain(advance.cyclic.iter().map(|cyclic| cyclic.duration))
        })
        .filter(|value| value.is_finite())
        .reduce(f32::max)
        .unwrap_or(0.0);
    Ok(EvaluatedRequest::RootMetadata(metadata))
}

/// Evaluate every request, mapping per-request failures (typically a clip whose animation the
/// creature does not ship) to `None` instead of failing the whole file. Roots whose contours
/// reference a failed evaluation are dropped later in `evaluated_speed_info`.
fn evaluate_recipe(
    recipe: &NeedsEvaluation,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Result<Vec<Option<EvaluatedRequest>>, SpeedInfoProducerError> {
    let started = std::time::Instant::now();
    let assets = evaluation_assets(recipe, roots, sapt_chain)?;
    let asset_seconds = started.elapsed().as_secs_f64();
    let evaluation_started = std::time::Instant::now();
    let results = recipe
        .requests
        .par_iter()
        .map(|request| {
            let result = match request {
                EvaluationRequest::Individual(request) => value_dir(&request.animation_path)
                    .map(|(speed, direction)| EvaluatedRequest::Individual { speed, direction }),
                EvaluationRequest::SpeedSampled(request) => {
                    evaluate_speed_sampled(request, &assets).ok()
                }
                EvaluationRequest::RootMetadata(request) => {
                    evaluate_root_metadata(request, &assets).ok()
                }
            };
            result
        })
        .collect();
    if started.elapsed().as_millis() >= 250 {
        eprintln!(
            "[animtext_speed] sapt={:?} requests={} workers={} assets={asset_seconds:.3}s evaluate={:.3}s",
            sapt_chain,
            recipe.requests.len(),
            rayon::current_num_threads(),
            evaluation_started.elapsed().as_secs_f64()
        );
    }
    Ok(results)
}

fn evaluated_request<'a>(
    evaluations: &'a [Option<EvaluatedRequest>],
    id: EvaluationRequestId,
) -> Result<&'a EvaluatedRequest, SpeedInfoProducerError> {
    evaluations
        .get(id.0 as usize)
        .and_then(Option::as_ref)
        .ok_or(SpeedInfoProducerError::MissingEvaluation(id))
}

fn evaluated_entry(
    entry: &RecipeEntry,
    evaluations: &[Option<EvaluatedRequest>],
) -> Result<ContourEntry, SpeedInfoProducerError> {
    match entry {
        RecipeEntry::Selector(entry) => Ok(entry.clone()),
        RecipeEntry::RootMetadata(id) => match evaluated_request(evaluations, *id)? {
            EvaluatedRequest::RootMetadata(value) => Ok(ContourEntry {
                state_id: -1,
                value: *value,
                link: None,
            }),
            _ => Err(SpeedInfoProducerError::Evaluation(format!(
                "request {} did not produce root metadata",
                id.0
            ))),
        },
    }
}

fn evaluated_contour(
    contour: &RecipeContour,
    recipe: &NeedsEvaluation,
    evaluations: &[Option<EvaluatedRequest>],
) -> Result<Contour, SpeedInfoProducerError> {
    match contour {
        RecipeContour::Collection { children, .. } => Ok(Contour::Collection(CollectionContour {
            children: children
                .iter()
                .map(|child| evaluated_contour(child, recipe, evaluations))
                .collect::<Result<Vec<_>, _>>()?,
        })),
        RecipeContour::Individual {
            parameter,
            clip,
            condition,
            entry,
            evaluation,
            ..
        } => match evaluated_request(evaluations, *evaluation)? {
            EvaluatedRequest::Individual { speed, direction } => {
                Ok(Contour::Individual(IndividualContour {
                    direction: *direction,
                    parameter: parameter.clone(),
                    speed: *speed,
                    clip: clip.clone(),
                    condition: condition.clone(),
                    entry: evaluated_entry(entry, evaluations)?,
                }))
            }
            _ => Err(SpeedInfoProducerError::Evaluation(format!(
                "request {} did not produce an Individual contour",
                evaluation.0
            ))),
        },
        RecipeContour::SpeedSampled {
            record,
            center_mode,
            clip,
            condition,
            entry,
            evaluation,
        } => match evaluated_request(evaluations, *evaluation)? {
            EvaluatedRequest::SpeedSampled {
                curves,
                directional_summary,
            } => {
                let capacity_hint = recipe
                    .records
                    .get(record.0 as usize)
                    .ok_or_else(|| {
                        SpeedInfoProducerError::Evaluation(format!(
                            "missing producer record {}",
                            record.0
                        ))
                    })?
                    .recipe_ordinal;
                let domain = match &recipe.requests[evaluation.0 as usize] {
                    EvaluationRequest::SpeedSampled(request) => &request.domain,
                    _ => unreachable!(),
                };
                Ok(Contour::SpeedSampled(SpeedSampledContour {
                    capacity_hint,
                    angle_min: domain.direction_min,
                    angle_max: domain.direction_max,
                    speed_min: domain.speed_min,
                    speed_max: domain.speed_max,
                    curves: curves.clone(),
                    directional_summary: directional_summary.clone(),
                    center_mode: *center_mode,
                    clip: clip.clone(),
                    condition: condition.clone(),
                    entry: evaluated_entry(entry, evaluations)?,
                }))
            }
            _ => Err(SpeedInfoProducerError::Evaluation(format!(
                "request {} did not produce a SpeedSampled contour",
                evaluation.0
            ))),
        },
    }
}

fn evaluated_speed_info(
    recipe: &NeedsEvaluation,
    evaluations: &[Option<EvaluatedRequest>],
) -> Result<SpeedInfoFile, SpeedInfoProducerError> {
    let mut roots = Vec::new();
    for root in &recipe.roots {
        let assembled = (|| {
            let contour = evaluated_contour(&root.contour, recipe, evaluations)?;
            let metadata = match root.metadata {
                RootMetadataRecipe::Collection {
                    center_mode,
                    evaluation,
                } => match evaluated_request(evaluations, evaluation)? {
                    EvaluatedRequest::RootMetadata(value) => {
                        RootMetadata::Collection(CollectionRootMetadata {
                            center_mode,
                            producer: ContourEntry {
                                state_id: -1,
                                value: *value,
                                link: None,
                            },
                        })
                    }
                    _ => {
                        return Err(SpeedInfoProducerError::Evaluation(format!(
                            "request {} did not produce Collection metadata",
                            evaluation.0
                        )));
                    }
                },
                RootMetadataRecipe::DirectIndividual { .. } => RootMetadata::DirectIndividual,
                RootMetadataRecipe::DirectSpeedSampled { .. } => RootMetadata::DirectSpeedSampled,
            };
            Ok(SpeedInfoRoot {
                state_machine_path: root.state_machine_path.clone(),
                contour,
                metadata,
            })
        })();
        match assembled {
            Ok(root) => roots.push(root),
            // A failed evaluation (clip the creature does not ship) drops only its root.
            Err(SpeedInfoProducerError::MissingEvaluation(_)) => continue,
            Err(other) => return Err(other),
        }
    }
    if roots.is_empty() {
        return Err(SpeedInfoProducerError::NoSpeedInfoTopology);
    }
    // Trim the entry chains to the depth the tree allows before anyone sees the file, so the
    // in-memory recipe matches the bytes that get written.
    for root in &mut roots {
        normalize_entry_links(&mut root.contour);
    }
    Ok(SpeedInfoFile { roots })
}

pub fn build_speed_info_body_weapon(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    if is_shared_mt_behavior(core_rel) {
        return build_speed_info_body_mt(core_rel, roots, sapt_chain);
    }
    let recipe = build_speed_info_recipe_weapon(core_rel, roots, sapt_chain).ok()?;
    let evaluations = evaluate_recipe(&recipe, roots, sapt_chain).ok()?;
    let speed_info = evaluated_speed_info(&recipe, &evaluations).ok()?;
    encode_speed_info(&speed_info).ok()
}

fn speed_info_leaf_basenames_mt(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> BTreeSet<String> {
    let g = BehaviorGraph::load_reachable(core_rel, roots);
    let mut out = BTreeSet::new();
    if g.files.is_empty() {
        return out;
    }
    let file = 0;
    for state_machine in 0..g.objs(file).len() {
        let object = &g.objs(file)[state_machine];
        if object.class_name != "hkbStateMachine"
            || selector_var_of_sm(object, g.objs(file), g.vars(file)).as_deref()
                != Some("iSyncIdleLocomotion")
        {
            continue;
        }
        let Some(tree) = build(&g, file, state_machine, Vec::new(), 0, false) else {
            continue;
        };
        let Some(resolved) = resolve_mt_locomotion(&tree, &g, sapt_chain) else {
            continue;
        };
        if matches!(resolved, ResolvedNode::Collection(_)) {
            collect_mt_leaf_clips(&tree, &g, sapt_chain, &mut out);
        }
    }
    out
}

/// The loop-clip basenames (lowercased, no ext) owned by this weapon subgraph's
/// `AnimationSpeedInfo` contour: the cyclic directional locomotion leaves. `AnimationOffsets`
/// must exclude these: the locomotion system owns their root motion, and caching them in
/// Offsets too makes the subgraph moonwalk. Empty for a subgraph with no SpeedInfo contour
/// (e.g. 1st-person `GunBehavior`), which then keeps its loops in Offsets. Uses the same
/// cross-file root selection as [`build_speed_info_body_weapon`]; keep the two in lockstep.
pub fn speed_info_leaf_basenames(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> BTreeSet<String> {
    if is_shared_mt_behavior(core_rel) {
        return speed_info_leaf_basenames_mt(core_rel, roots, sapt_chain);
    }
    let g = BehaviorGraph::load_reachable(core_rel, roots);
    let mut out: BTreeSet<String> = BTreeSet::new();
    for f in 0..g.files.len() {
        let objects = g.objs(f);
        if g.files[f].core.graph_name.is_empty() {
            continue;
        }
        let mut camera_subtree: HashSet<usize> = HashSet::new();
        if let Some(cam) = objects.iter().position(|o| {
            o.class_name == "hkbStateMachine"
                && string_member(o, "name").as_deref() == Some("CameraStateMachine")
        }) {
            collect_subtree(cam, objects, &mut camera_subtree);
        }
        for idx in 0..objects.len() {
            let obj = &objects[idx];
            if obj.class_name != "hkbStateMachine" {
                continue;
            }
            if selector_var_of_sm(obj, objects, g.vars(f)).as_deref() != Some("iSyncIdleLocomotion")
            {
                continue;
            }
            if camera_subtree.contains(&idx) {
                continue;
            }
            if let Some(source) = tiered::source(&g, f, idx, sapt_chain) {
                out.extend(source.clips.iter().filter_map(|clip| {
                    string_member(&objects[*clip], "animationName").map(|name| leaf_basename(&name))
                }));
                continue;
            }
            if let Some(tree) = build(&g, f, idx, Vec::new(), 0, false) {
                collect_leaf_clips(&tree, &g, sapt_chain, &mut out);
            }
        }
    }
    out
}

/// Collect the loop-clip basename of each MT contour leaf bound to `walkSpeedMult`,
/// `jogSpeedMult` or `runSpeedMult`; the MT counterpart of [`collect_leaf_clips`].
fn collect_mt_leaf_clips(
    node: &Node,
    g: &BehaviorGraph,
    sapt_chain: &[String],
    out: &mut BTreeSet<String>,
) {
    match node {
        Node::Collection(children) => {
            for child in children {
                collect_mt_leaf_clips(child, g, sapt_chain, out);
            }
        }
        Node::Individual(leaf) => {
            if !matches!(
                leaf.param.to_ascii_lowercase().as_str(),
                "walkspeedmult" | "jogspeedmult" | "runspeedmult"
            ) {
                return;
            }
            let (file, clip) = leaf.speed_clip;
            if let Some(path) = clip_loop_path(&g.objs(file)[clip], g.roots, sapt_chain) {
                out.insert(leaf_basename(&path.to_string_lossy()));
            }
        }
    }
}

/// Walk a contour tree collecting each `Individual` leaf's loop-clip basename, but only for
/// leaves bound to an `fLocomotion*PlaybackSpeed` variable (the cyclic directional walk/run/
/// sneak loops). The selection SMs also carry fire/reload clips bound to `weaponSpeedMult`/
/// `reloadSpeedMult`; CK keeps those (`wpnreload`/`wpnfireauto*`) in the offsets cache, so
/// they must not be excluded.
fn collect_leaf_clips(
    node: &Node,
    g: &BehaviorGraph,
    sapt_chain: &[String],
    out: &mut BTreeSet<String>,
) {
    match node {
        Node::Collection(ch) => {
            for c in ch {
                collect_leaf_clips(c, g, sapt_chain, out);
            }
        }
        Node::Individual(leaf) => {
            let p = leaf.param.to_ascii_lowercase();
            if !(p.starts_with("flocomotion") && p.ends_with("playbackspeed")) {
                return;
            }
            let (cf, ci) = leaf.speed_clip;
            if let Some(path) = clip_loop_path(&g.objs(cf)[ci], g.roots, sapt_chain) {
                out.insert(leaf_basename(&path.to_string_lossy()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    fn base_meshes() -> PathBuf {
        repo_root().join("extracted/fo4/Meshes")
    }

    /// A `MODE_SINGLE_PLAY` clip named as a blender's sync master must not freeze the locomotion
    /// blended underneath it. In `LocomotionBlendGunUpBoltChargeBlend`, child 0 is the one-shot
    /// `WPNBoltChargeReady` (`indexOfSyncMasterChild` 0) and child 1 the locomotion tree; adopting
    /// the clamped master's phase zeroes every `RifleBoltChargeState_SM` contour, and the
    /// character sticks mid-stride. Vanilla ships 6554 of these contours, none all-zero.
    #[test]
    fn bolt_charge_locomotion_survives_its_single_play_sync_master() {
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\WeaponBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file() {
            eprintln!("base character fixtures absent; skipping");
            return;
        }
        let chain = [
            r"Actors\Character\Animations\Weapon\44Pistol\Player",
            r"Actors\Character\Animations\Weapon\44Pistol",
            r"Actors\Character\Animations\Weapon\GripRifleStraight",
            r"Actors\Character\Animations\Paired",
            r"Actors\Character\Animations",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [base.as_path()];

        let body =
            build_speed_info_body_weapon(core, &roots, &chain).expect("44Pistol weapon SpeedInfo");
        let generated = decode_speed_info(&body).expect("generated SpeedInfo decodes");

        fn sampled<'a>(contour: &'a Contour, out: &mut Vec<&'a SpeedSampledContour>) {
            match contour {
                Contour::Collection(collection) => {
                    for child in &collection.children {
                        sampled(child, out);
                    }
                }
                Contour::SpeedSampled(node) => out.push(node),
                Contour::Individual(_) => {}
            }
        }

        let mut contours = Vec::new();
        for root in &generated.roots {
            if root.state_machine_path.ends_with("RifleBoltChargeState_SM") {
                sampled(&root.contour, &mut contours);
            }
        }
        assert!(
            !contours.is_empty(),
            "no sampled contour produced for RifleBoltChargeState_SM"
        );

        let frozen = contours
            .iter()
            .filter(|node| {
                node.curves
                    .iter()
                    .all(|curve| curve.samples.iter().all(|s| s.output.abs() < 1.0e-6))
            })
            .count();
        assert_eq!(
            frozen,
            0,
            "{frozen} of {} bolt-charge contours report zero speed in every direction — the              single-play sync master is freezing the locomotion blend again",
            contours.len()
        );
    }

    #[test]
    fn pepper_shaker_third_person_speed_info_is_generated() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\WeaponBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted
                .join("Actors/Character/animations/weapon/peppershakershotgun")
                .is_dir()
        {
            eprintln!("converted Pepper Shaker/base character fixtures absent; skipping");
            return;
        }
        let chain = [
            r"Actors\Character\Animations\Weapon\PepperShakerShotgun",
            r"Actors\Character\Animations\Weapon\Minigun\Player",
            r"Actors\Character\Animations\Weapon\Minigun",
            r"Actors\Character\Animations\Weapon\GripHeavy\Player",
            r"Actors\Character\Animations\Weapon\GripHeavy",
            r"Actors\Character\Animations\Paired",
            r"Actors\Character\Animations\Synth",
            r"Actors\Character\Animations",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [converted.as_path(), base.as_path()];

        let body = build_speed_info_body_weapon(core, &roots, &chain)
            .expect("Pepper Shaker third-person SpeedInfo");
        let generated = decode_speed_info(&body).expect("generated SpeedInfo decodes");
        let stats = generated.stats();
        assert!(stats.roots > 0);
        assert!(stats.individuals > 0);
        assert!(stats.speed_sampled > 0);
        assert!(!speed_info_leaf_basenames(core, &roots, &chain).is_empty());
    }

    /// Two weapons on the same core but different grip chains must not share a speed contour.
    /// Vanilla never does: 66 FO4 weapon subgraphs share an identical 24-contour-root set and all
    /// 66 differ in bytes, because the contour samples each grip's own loop clips.
    ///
    /// Chains are the live ones from `HumanRaceAdditivePluginPort`: AlienRifle (`GripRifleStraight`
    /// tail) and GaussPistol (`Pistol` tail), on a pinned core. Leaf basenames are printed but not
    /// asserted: every grip ships identically named clips (`wpnrunforwardready` etc.) in its own
    /// folder, so only the emitted body discriminates.
    #[test]
    fn weapon_speed_contour_varies_with_grip_chain() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\WeaponBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted
                .join("Actors/Character/Animations/FO76/Weapon/AlienRifle")
                .is_dir()
            || !converted
                .join("Actors/Character/Animations/FO76/Weapon/GaussPistol")
                .is_dir()
        {
            eprintln!("converted AlienRifle/GaussPistol/base character fixtures absent; skipping");
            return;
        }
        let rifle_chain = [
            r"Actors\Character\Animations\FO76\Weapon\AlienRifle",
            r"Actors\Character\Animations\Weapon\AlienRifle",
            r"Actors\Character\Animations\FO76\Weapon\CombatShotgun\Player",
            r"Actors\Character\Animations\Weapon\CombatShotgun\Player",
            r"Actors\Character\Animations\FO76\Weapon\CombatShotgun",
            r"Actors\Character\Animations\Weapon\CombatShotgun",
            r"Actors\Character\Animations\FO76\Weapon\GripRifleStraight\Player",
            r"Actors\Character\Animations\Weapon\GripRifleStraight\Player",
            r"Actors\Character\Animations\FO76\Weapon\GripRifleStraight",
            r"Actors\Character\Animations\Weapon\GripRifleStraight",
            r"Actors\Character\Animations\Common\Emotes",
            r"Actors\Character\Animations\Common",
            r"Actors\Character\Animations\Player",
            r"Actors\Character\Animations",
            r"Actors\Character\Animations\Paired",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let pistol_chain = [
            r"Actors\Character\Animations\FO76\Weapon\GaussPistol",
            r"Actors\Character\Animations\Weapon\GaussPistol",
            r"Actors\Character\Animations\FO76\Weapon\Pistol\Injured\Right",
            r"Actors\Character\Animations\Weapon\Pistol\Injured\Right",
            r"Actors\Character\Animations\FO76\Weapon\Pistol\Player",
            r"Actors\Character\Animations\Weapon\Pistol\Player",
            r"Actors\Character\Animations\FO76\Weapon\Pistol",
            r"Actors\Character\Animations\Weapon\Pistol",
            r"Actors\Character\Animations\FO76\Weapon\Rifle\Neutral",
            r"Actors\Character\Animations\Weapon\Rifle\Neutral",
            r"Actors\Character\Animations\Paired",
            r"Actors\Character\Animations\Common\Emotes",
            r"Actors\Character\Animations\Common",
            r"Actors\Character\Animations\Player",
            r"Actors\Character\Animations",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [converted.as_path(), base.as_path()];

        let rifle_leaves = speed_info_leaf_basenames(core, &roots, &rifle_chain);
        let pistol_leaves = speed_info_leaf_basenames(core, &roots, &pistol_chain);
        eprintln!(
            "rifle leaves={} pistol leaves={} rifle-only={:?} pistol-only={:?}",
            rifle_leaves.len(),
            pistol_leaves.len(),
            rifle_leaves.difference(&pistol_leaves).collect::<Vec<_>>(),
            pistol_leaves.difference(&rifle_leaves).collect::<Vec<_>>(),
        );

        let rifle = build_speed_info_body_weapon(core, &roots, &rifle_chain)
            .expect("straight-rifle grip SpeedInfo");
        let pistol = build_speed_info_body_weapon(core, &roots, &pistol_chain)
            .expect("pistol grip SpeedInfo");
        eprintln!("rifle body={}B pistol body={}B", rifle.len(), pistol.len());
        assert_ne!(
            rifle, pistol,
            "straight-rifle and pistol grips emitted a byte-identical speed contour; \
             the SAPT chain is not reaching the sampled clips"
        );
    }

    /// The forward fan every CK golden ships spans a hair under 12 steps of PI/12 yet still gets
    /// 13 curves, while the neighbouring rear fan genuinely has 12.
    #[test]
    fn knife_edge_direction_fan_keeps_its_last_curve() {
        let step = f32::from_bits(0x3e86_0a92);
        assert_eq!(direction_curve_count(-1.5770791, 1.5645132, step), 13);
        assert_eq!(direction_curve_count(2.3624778, 5.4915042, step), 12);
        assert_eq!(direction_curve_count(-0.7916815, 2.3624778, step), 13);
        assert_eq!(
            direction_curve_count(-std::f32::consts::PI, std::f32::consts::PI, step),
            25
        );
    }

    /// A 3P weapon speed state nests a direction state machine whose sectors CK emits as a pair
    /// inside their own collection (vanilla: 41277 paired roots, 0 degraded). Taking the nested
    /// SM's forward blender alone drops the `moveBackward` sector (135°–315°, every strafe-right
    /// and rearward angle), so the weapon cannot strafe.
    #[test]
    fn weapon_sampled_contours_are_emitted_as_a_direction_pair() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\WeaponBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted
                .join("Actors/Character/Animations/Weapon/PlasmaCaster")
                .is_dir()
        {
            eprintln!("converted PlasmaCaster / base character fixtures absent; skipping");
            return;
        }
        // The real chain off `AnimsPlasmaCaster`'s 3P block, verbatim from the converted RACE.
        let chain = [
            r"Actors\Character\Animations\FO76\Weapon\PlasmaCaster",
            r"Actors\Character\Animations\Weapon\PlasmaCaster",
            r"Actors\Character\Animations\Weapon\GatlingPlasma",
            r"Actors\Character\Animations\Weapon\GripHeavy",
            r"Actors\Character\Animations\Common\Emotes",
            r"Actors\Character\Animations\Common",
            r"Actors\Character\Animations\Player",
            r"Actors\Character\Animations\Paired",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [converted.as_path(), base.as_path()];
        let recipe = build_speed_info_recipe_weapon(core, &roots, &chain)
            .expect("PlasmaCaster 3P weapon locomotion recipe");

        fn walk(
            node: &RecipeContour,
            paired: &mut usize,
            bare: &mut usize,
            conditions: &mut Vec<String>,
        ) {
            match node {
                RecipeContour::Collection { children, .. } => {
                    let sampled = children
                        .iter()
                        .filter(|child| matches!(child, RecipeContour::SpeedSampled { .. }))
                        .count();
                    if sampled > 1 {
                        *paired += 1;
                    } else if sampled == 1 {
                        *bare += 1;
                    }
                    for child in children {
                        walk(child, paired, bare, conditions);
                    }
                }
                RecipeContour::SpeedSampled {
                    condition,
                    clip,
                    center_mode,
                    ..
                } => conditions.push(format!("{condition}/{clip}/{center_mode:?}")),
                RecipeContour::Individual { .. } => {}
            }
        }

        let (mut paired, mut bare, mut conditions) = (0usize, 0usize, Vec::new());
        for root in &recipe.roots {
            walk(&root.contour, &mut paired, &mut bare, &mut conditions);
        }

        assert!(
            recipe.stats().speed_sampled > 0,
            "no SpeedSampled contour emitted at all"
        );
        assert!(
            paired > 0,
            "every sampled contour is unpaired ({bare} bare, 0 paired) — the single-sector \
             fallback ran ahead of the nested direction scan again; conditions {conditions:?}"
        );
        assert!(
            conditions
                .iter()
                .any(|condition| condition.contains("Direction")),
            "no sampled contour is conditioned on a direction selector; got {conditions:?} — \
             `iLocomotionSpeedState` here is the fallback branch's fingerprint"
        );

        // `RelaxedWalkBackward` is the discriminating sector. Its child weights are asymmetric,
        // so the mirrored `PI/2 - TAU * weight` fan collapsed it to a zero-centred
        // [-3.1353, 0.0063] instead of the pi-centred [1.5645, 4.7061] vanilla ships.
        // `CombatWalk*` cannot catch that: its weights are symmetric about 0.125, so the mirror
        // maps the fan onto itself and both formulas agree.
        let ranges = recipe
            .requests
            .iter()
            .filter_map(|request| match request {
                EvaluationRequest::SpeedSampled(sampled) => {
                    Some((sampled.domain.direction_min, sampled.domain.direction_max))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            ranges
                .iter()
                .any(|(low, high)| (low - 1.5645).abs() < 1e-3 && (high - 4.7061).abs() < 1e-3),
            "no rearward sector spans vanilla's [1.5645, 4.7061]; got {ranges:?}"
        );

        // The speed state's trailer, which vanilla carries on every sampled group and we emitted
        // on none: `walkRunBlendStart` conditioned on `iLocomotionSpeedState`. Its absence is
        // exactly the "our root == vanilla's root minus one contour" shape, and the slow walk.
        fn links(node: &RecipeContour, out: &mut Vec<String>) {
            match node {
                RecipeContour::Collection { children, .. } => {
                    for child in children {
                        links(child, out);
                    }
                }
                RecipeContour::SpeedSampled {
                    entry: RecipeEntry::Selector(entry),
                    ..
                } => {
                    if let Some(link) = &entry.link {
                        out.push(format!("{}/{}", link.event, link.variable));
                    }
                }
                _ => {}
            }
        }
        let mut trailers = Vec::new();
        for root in &recipe.roots {
            links(&root.contour, &mut trailers);
        }
        assert!(
            trailers.iter().any(|t| t.starts_with("walkRunBlendStart/")),
            "no sampled group carries the speed-state trailer; got {trailers:?}"
        );
    }

    /// A humanoid creature that lacks one of the clips a weapon-behavior root replays
    /// (MoleMiner ships no `WPNMineThrow`) must still produce SpeedInfo for every root whose
    /// animations resolve — one missing clip drops only its own root, never the whole file.
    /// This is the gun-stance locomotion contour: without it converted mole miners chase at
    /// walk speed and never run.
    #[test]
    fn humanoid_creature_gun_locomotion_speed_info_survives_missing_clips() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\WeaponBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted
                .join("Actors/MoleMiner/Animations/GripAssault")
                .is_dir()
        {
            eprintln!("converted MoleMiner / base character fixtures absent; skipping");
            return;
        }
        if converted
            .join("Actors/MoleMiner/Animations/GripAssault/wpnminethrow.hkx")
            .is_file()
        {
            // The alias-synthesis fixup ships a WPNMineThrow clip, so this test's
            // missing-clip premise does not hold on the live tree.
            eprintln!("live tree ships a synthesized WPNMineThrow alias; skipping");
            return;
        }
        let roots = [converted.as_path(), base.as_path()];
        let chain: Vec<String> = [
            r"Actors\MoleMiner\Animations\GripAssault",
            r"Actors\MoleMiner\Animations\Shared",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();

        let body = build_speed_info_body_weapon(core, &roots, &chain)
            .expect("MoleMiner GripAssault SpeedInfo despite missing WPNMineThrow");
        let generated = decode_speed_info(&body).unwrap();
        let stats = generated.stats();
        assert!(stats.roots > 0);
        assert!(stats.individuals > 0);
        assert!(
            generated
                .roots
                .iter()
                .any(|root| root.state_machine_path.ends_with("RifleReady_SM")),
            "ready-stance locomotion root must survive"
        );
        assert!(
            generated
                .roots
                .iter()
                .all(|root| !root.state_machine_path.ends_with("DefaultMineThrow_SM")),
            "the root that replays the missing WPNMineThrow clip is dropped, not kept broken"
        );

        // The arm-injured wrappers must produce too: their SAPT chains lead with
        // `<Weapon>\Injured\<Side>` dirs the creature does not ship, falling through to the
        // weapon dir. These are 10 of MoleMiner's 19 starved subgraphs.
        let injured_chain: Vec<String> = [
            r"Actors\MoleMiner\Animations\RailwayRifle\Injured\Left",
            r"Actors\MoleMiner\Animations\RailwayRifle",
            r"Actors\MoleMiner\Animations\Shared",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        let body = build_speed_info_body_weapon(
            r"Actors\Character\Behaviors\LeftArmInjuredWeaponWrappingBehavior.hkx",
            &roots,
            &injured_chain,
        )
        .expect("MoleMiner injured-wrapper SpeedInfo");
        let stats = decode_speed_info(&body).unwrap().stats();
        assert!(stats.roots > 0);
        assert!(stats.individuals > 0);
    }

    /// The vanilla FO4 SuperMutant MT subgraph is the role-0 oracle for humanoid creatures using
    /// the same shared `MTBehavior.hkx`. Its three Collection roots own Walk/Jog/Run; the other
    /// four roots are sampled sneak/swim contours. The weapon producer recovers only a camera
    /// root from this graph and nothing from MoleMiner's SAPT chain, leaving every locomotion loop
    /// in `AnimationOffsets` and the converted creature walk-only.
    #[test]
    fn humanoid_creature_mt_locomotion_speed_info_is_generated() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\MTBehavior.hkx";
        let oracle_path = repo_root()
            .join("extracted/fo4/meshes/AnimTextData/animationspeedinfo/8988378211353393101.txt");
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted.join("Actors/MoleMiner/animations/MT").is_dir()
            || !oracle_path.is_file()
        {
            eprintln!("converted MoleMiner / base character / oracle fixtures absent; skipping");
            return;
        }

        // 1. Reproduce the vanilla oracle from its own base-game inputs.
        let base_roots = [base.as_path()];
        let supermutant_chain = [
            r"Actors\Supermutant\Animations\MT\Neutral",
            r"Actors\Supermutant\Animations\H2H",
            r"Actors\Supermutant\Animations\Shared",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let oracle_bytes = std::fs::read(&oracle_path).unwrap();
        let oracle = decode_speed_info(&oracle_bytes).expect("vanilla MT SpeedInfo decodes");
        let generated = build_speed_info_body_weapon(core, &base_roots, &supermutant_chain)
            .expect("SuperMutant MT SpeedInfo");
        let generated = decode_speed_info(&generated).unwrap();
        assert_eq!(
            generated.stats(),
            ContourStats {
                roots: 3,
                collections: 3,
                individuals: 9,
                speed_sampled: 0,
            }
        );
        for root in &generated.roots {
            let oracle_root = oracle
                .roots
                .iter()
                .find(|candidate| candidate.state_machine_path == root.state_machine_path)
                .expect("generated MT movement root exists in vanilla oracle");
            let (Contour::Collection(generated), Contour::Collection(oracle)) =
                (&root.contour, &oracle_root.contour)
            else {
                panic!("MT movement roots must be Collections");
            };
            let (
                RootMetadata::Collection(generated_metadata),
                RootMetadata::Collection(oracle_metadata),
            ) = (&root.metadata, &oracle_root.metadata)
            else {
                panic!("MT movement roots must carry Collection metadata");
            };
            assert!(
                (generated_metadata.producer.value - oracle_metadata.producer.value).abs()
                    < 0.000001,
                "{} metadata: generated {:?}, oracle {:?}",
                root.state_machine_path,
                generated_metadata.producer.value,
                oracle_metadata.producer.value,
            );
            assert_eq!(generated.children.len(), oracle.children.len());
            for (generated, oracle) in generated.children.iter().zip(&oracle.children) {
                let (Contour::Individual(generated), Contour::Individual(oracle)) =
                    (generated, oracle)
                else {
                    panic!("MT movement children must be Individuals");
                };
                assert_eq!(generated.parameter, oracle.parameter);
                assert_eq!(generated.clip, oracle.clip);
                assert_eq!(generated.condition, oracle.condition);
                assert_eq!(generated.entry, oracle.entry);
                assert!((generated.speed - oracle.speed).abs() < 0.001);
                assert!(
                    generated
                        .direction
                        .iter()
                        .zip(oracle.direction)
                        .all(|(left, right)| (left - right).abs() < 0.000001)
                );
            }
        }

        // 2. The converted humanoid creature must then produce its own contour.
        let chain = [
            r"Actors\MoleMiner\Animations\MT",
            r"Actors\MoleMiner\Animations\H2H",
            r"Actors\MoleMiner\Animations\Shared",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [converted.as_path(), base.as_path()];
        let body = build_speed_info_body_weapon(core, &roots, &chain)
            .expect("MoleMiner MT locomotion SpeedInfo");
        assert!(decode_speed_info(&body).unwrap().stats().individuals > 0);
        assert!(
            speed_info_leaf_basenames(core, &roots, &chain)
                .iter()
                .any(|clip| clip == "runforward")
        );

        let scorched_chain = [
            r"Actors\Scorched\Animations\H2H",
            r"Actors\Character\Animations\MT\Neutral",
            r"Actors\Character\Animations",
            r"Actors\Character\Animations\Common",
            r"Actors\Scorched\Animations\Mouth",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let body = build_speed_info_body_weapon(core, &roots, &scorched_chain)
            .expect("Scorched MT locomotion SpeedInfo");
        assert!(decode_speed_info(&body).unwrap().stats().individuals > 0);
        assert!(
            speed_info_leaf_basenames(core, &roots, &scorched_chain)
                .iter()
                .any(|clip| clip == "runforward")
        );
    }

    #[test]
    fn humanoid_creature_melee_locomotion_speed_info_is_generated() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\MeleeBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted.join("Actors/MoleMiner/animations/H2H").is_dir()
        {
            eprintln!("converted MoleMiner / base character fixtures absent; skipping");
            return;
        }
        let chain = [
            r"Actors\MoleMiner\Animations\H2H",
            r"Actors\MoleMiner\Animations\Shared",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [converted.as_path(), base.as_path()];

        let recipe = build_speed_info_recipe_weapon(core, &roots, &chain)
            .expect("MoleMiner H2H locomotion recipe");
        let evaluations =
            evaluate_recipe(&recipe, &roots, &chain).expect("MoleMiner H2H locomotion evaluations");
        let generated = evaluated_speed_info(&recipe, &evaluations)
            .expect("MoleMiner H2H locomotion SpeedInfo");
        assert!(generated.stats().individuals > 0);
        assert!(
            speed_info_leaf_basenames(core, &roots, &chain)
                .iter()
                .any(|clip| clip == "runforward")
        );

        let scorched_chain = [
            r"Actors\Scorched\Animations\H2H",
            r"Actors\Character\Animations\H2H",
            r"Actors\Character\Animations\1HM",
            r"Actors\Character\Animations\Common",
            r"Actors\Character\Animations",
            r"Actors\Scorched\Animations\Mouth",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let recipe = build_speed_info_recipe_weapon(core, &roots, &scorched_chain)
            .expect("Scorched H2H locomotion recipe");
        let evaluations = evaluate_recipe(&recipe, &roots, &scorched_chain)
            .expect("Scorched H2H locomotion evaluations");
        let generated =
            evaluated_speed_info(&recipe, &evaluations).expect("Scorched H2H locomotion SpeedInfo");
        assert!(generated.stats().individuals > 0);
        assert!(
            speed_info_leaf_basenames(core, &roots, &scorched_chain)
                .iter()
                .any(|clip| clip == "runforward")
        );
    }

    /// A melee locomotion contour must carry a `SpeedSampled` contour, not just per-clip
    /// individuals. `Actor::ModifyMovementTypeBasedOnAnimationState` clamps `Speeds[dir][JOG]`
    /// and `Speeds[dir][RUN]` down to the active contour; the sampled array is what advertises
    /// the graph's reachable speed range. Without it the run slot is pinned to the walk clip's
    /// root-motion speed and the actor chases at walk pace on `NPC_Melee_MT`. Vanilla ships
    /// 10966 sampled contours across its 242 `MeleeBehavior.hkb` files.
    #[test]
    fn scorched_melee_contour_carries_sampled_speed_range() {
        let converted = repo_root().join("mods/SeventySix/data/Meshes");
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\MeleeBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file()
            || !converted.join("Actors/Scorched/animations/1HM").is_dir()
        {
            eprintln!("converted Scorched / base character fixtures absent; skipping");
            return;
        }
        // The 1HM melee block off `ScorchedRace`, which is what a pipe-wrench Scorched binds.
        let chain = [
            r"Actors\Scorched\Animations\1HM",
            r"Actors\Scorched\Animations\1HM\Attack",
            r"Actors\Character\Animations\1HM",
            r"Actors\Character\Animations\1HM\Attack",
            r"Actors\Character\Animations\Common",
            r"Actors\Scorched\Animations\Mouth",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let roots = [converted.as_path(), base.as_path()];
        let recipe = build_speed_info_recipe_weapon(core, &roots, &chain)
            .expect("Scorched 1HM melee locomotion recipe");
        let emitted = recipe
            .roots
            .iter()
            .map(|root| root.state_machine_path.as_str())
            .collect::<Vec<_>>();
        assert!(
            recipe.stats().speed_sampled > 0,
            "no SpeedSampled contour; emitted {emitted:?}"
        );
        // Vanilla's melee sampled contour spans `Direction` x `Speed` and carries a directional
        // summary; an empty summary is the shape the base-weapon producer already guards against.
        assert!(recipe.requests.iter().any(|request| matches!(
            request,
            EvaluationRequest::SpeedSampled(sampled)
                if sampled.domain.direction_variable == "Direction"
                    && sampled.domain.speed_variable == "Speed"
        )));
        assert!(recipe.requests.iter().all(|request| !matches!(
            request,
            EvaluationRequest::SpeedSampled(sampled) if sampled.directional_summary.is_empty()
        )));
    }

    #[test]
    #[ignore = "diagnostic"]
    fn survey_all_melee_sampled_ranges() {
        for (label, rel) in [
            (
                "OURS",
                "mods/SeventySix/data/Meshes/AnimTextData/AnimationSpeedInfo",
            ),
            (
                "VANILLA",
                "extracted/fo4/meshes/AnimTextData/animationspeedinfo",
            ),
        ] {
            let dir = repo_root().join(rel);
            let mut files = 0usize;
            let mut without_sampled = 0usize;
            let mut worst: Option<(f32, String)> = None;
            let mut best: Option<f32> = None;
            for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                let bytes = std::fs::read(entry.path()).unwrap();
                if !bytes.windows(17).any(|w| w == b"MeleeBehavior.hkb") {
                    continue;
                }
                files += 1;
                let Ok(info) = decode_speed_info(&bytes) else {
                    continue;
                };
                let mut maxima = Vec::new();
                for root in &info.roots {
                    let mut stack = vec![&root.contour];
                    while let Some(c) = stack.pop() {
                        match c {
                            Contour::Collection(col) => stack.extend(col.children.iter()),
                            Contour::SpeedSampled(s) => {
                                let reach = s
                                    .curves
                                    .iter()
                                    .flat_map(|curve| curve.samples.iter())
                                    .map(|pair| pair.output)
                                    .fold(f32::NEG_INFINITY, f32::max);
                                maxima.push(reach);
                            }
                            _ => {}
                        }
                    }
                }
                if maxima.is_empty() {
                    without_sampled += 1;
                    continue;
                }
                let file_max = maxima.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                if worst.as_ref().is_none_or(|(w, _)| file_max < *w) {
                    worst = Some((file_max, entry.file_name().to_string_lossy().into_owned()));
                }
                best = Some(best.map_or(file_max, |b: f32| b.max(file_max)));
            }
            eprintln!(
                "{label}: {files} melee files, {without_sampled} WITHOUT any sampled contour; \
                 reachable-output max: worst-file={:?} best-file={:?}",
                worst, best
            );
        }
    }

    #[test]
    fn base_weapon_recipe_uses_structural_topology_without_gauss_assets() {
        let base = base_meshes();
        let core = r"Actors\Character\Behaviors\NoHandIKRelaxedWeaponWrappingBehavior.hkx";
        if !base.join(core.replace('\\', "/")).is_file() {
            eprintln!("base weapon fixtures absent; skipping");
            return;
        }
        let chain = [
            r"Actors\Character\Animations\Weapon\Rifle\Neutral",
            r"Actors\Character\Animations\Weapon\Pistol",
            r"Actors\Character\Animations",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let recipe = build_speed_info_recipe_weapon(core, &[base.as_path()], &chain).unwrap();
        assert!(!recipe.roots.is_empty());
        assert!(recipe.stats().speed_sampled > 0);
        assert!(recipe.requests.iter().any(|request| matches!(
            request,
            EvaluationRequest::SpeedSampled(sampled)
                if sampled.domain.direction_variable == "Direction"
                    && sampled.domain.speed_variable == "Speed"
        )));
        assert!(recipe
            .requests
            .iter()
            .all(|request| !matches!(request, EvaluationRequest::SpeedSampled(sampled) if sampled.directional_summary.is_empty())));
    }
}
