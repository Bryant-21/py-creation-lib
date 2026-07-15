//! AnimationSpeedInfo contour codec and CK producer topology.
//!
//! Creature contours remain directly generative from clip root motion. Weapon contours use the
//! CK intermediate-record model and stop at a typed evaluation recipe; final bytes are withheld
//! until an independent behavior evaluator supplies sampled surfaces and producer metadata.
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
//! and a recursive selector-path `entry`. (Full field RE: `speedinfo_generate.md` §1-5.)
//!
//! ## Locomotion-SM selection (the `sm_path` seed — RE'd here)
//!
//! The `sm_path` seed is derived by walking
//! the **default-state (`startStateId`) chain** from the behavior graph's `rootGenerator`
//! through generator wrappers (layer/modifier/selector) and SMs, and taking the **deepest SM
//! whose subtree still contains a speed-bound clip**. The chain follows each SM's default state,
//! so a non-default combat sibling is naturally excluded; it stops at the first no-speed SM (a
//! creature's idle SM, e.g. Snallygaster `StandingStateMachine`), leaving its parent — the
//! locomotion SM (`IdleLocomotion_SM`) — as the seed. The seed's contour collapses unary down to
//! the first branching SM (`WalkRunJog_NonStrafing_SM`), the root `Collection`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use havok_native::behavior_eval::{
    AnimationPackfile, BehaviorEvaluator, LoadOptions, RootMotionProjection,
    VariableValue as EvaluatorVariableValue,
};
use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

use super::offsets::extract_baked_reference_frame;

mod contour;
mod producer;

pub use contour::{
    CenterMode, CollectionContour, CollectionRootMetadata, Contour, ContourCodecError,
    ContourStats, DirectionCurve, Entry as ContourEntry, EntryLink as ContourEntryLink,
    IndividualContour, RootMetadata, SamplePair, SpeedInfoFile, SpeedInfoRoot, SpeedSampledContour,
    decode_speed_info, encode_speed_info,
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
// `hkbBehaviorReferenceGenerator` whose `behaviorName` names a SEPARATE behavior file
// (`WeaponBehavior` → e.g. a directional-locomotion behavior). Each file has its own
// variableBindingSet/event/`hkbBehaviorGraph.name` index space, so a merged object graph
// would be wrong — instead every object reference travels as a `(file, idx)` pair and each
// file keeps its own string data. This is the shared cross-file primitive (also needed by
// §6b offsets). A single-file graph (one `Behavior`) reproduces the creature path exactly.
// ---------------------------------------------------------------------------------------

/// One parsed behavior file plus its per-file string-index spaces.
struct Behavior {
    objects: Vec<HkxObject>,
    var_names: Vec<String>,
    event_names: Vec<String>,
    graph_name: String,
    root_generator: Option<usize>,
    /// Normalized (`\`-sep) relpath this file was reached by — `behaviorName`s are resolved
    /// relative to its parent dir. Empty for a single-file (creature) graph.
    rel: String,
}

struct BehaviorGraph<'a> {
    files: Vec<Behavior>,
    by_rel: HashMap<String, usize>, // lowercased `\`-rel → file id
    roots: &'a [&'a Path],
}

impl<'a> BehaviorGraph<'a> {
    fn objs(&self, f: usize) -> &[HkxObject] {
        &self.files[f].objects
    }
    fn vars(&self, f: usize) -> &[String] {
        &self.files[f].var_names
    }
    fn evs(&self, f: usize) -> &[String] {
        &self.files[f].event_names
    }

    /// `behaviorName` (relative to `from`'s parent dir) → loaded file id, if reachable.
    fn resolve(&self, from: usize, behavior_name: &str) -> Option<usize> {
        let rel = join_behavior_rel(&behavior_parent_dir(&self.files[from].rel), behavior_name);
        self.by_rel
            .get(&rel.replace('/', "\\").to_ascii_lowercase())
            .copied()
    }

    fn make(objects: Vec<HkxObject>, rel: String) -> Behavior {
        let (var_names, event_names) = collect_string_data(&objects);
        let graph = objects.iter().find(|o| o.class_name == "hkbBehaviorGraph");
        let graph_name = graph
            .and_then(|o| string_member(o, "name"))
            .unwrap_or_default();
        let root_generator = graph.and_then(|o| first_ptr(o, "rootGenerator"));
        Behavior {
            objects,
            var_names,
            event_names,
            graph_name,
            root_generator,
            rel,
        }
    }

    /// Single-file graph (creature path): just the core, no reference following.
    fn load_single(core_disk: &Path, roots: &'a [&'a Path]) -> Option<Self> {
        let data = std::fs::read(core_disk).ok()?;
        let objects = read_packfile(&data).ok()?.objects().to_vec();
        Some(BehaviorGraph {
            files: vec![Self::make(objects, String::new())],
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
            let Ok(data) = std::fs::read(&disk) else {
                continue;
            };
            let Ok(hkx) = read_packfile(&data) else {
                continue;
            };
            let objects = hkx.objects().to_vec();
            let parent = behavior_parent_dir(&rel);
            for o in &objects {
                if o.class_name == "hkbBehaviorReferenceGenerator" {
                    if let Some(name) = string_member(o, "behaviorName") {
                        queue.push_back(join_behavior_rel(&parent, &name));
                    }
                }
            }
            let fid = g.files.len();
            g.by_rel.insert(key, fid);
            g.files.push(Self::make(objects, rel));
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
            let data = std::fs::read(&disk)
                .map_err(|_| SpeedInfoProducerError::BehaviorDecode(rel.clone()))?;
            let hkx = read_packfile(&data)
                .map_err(|_| SpeedInfoProducerError::BehaviorDecode(rel.clone()))?;
            let objects = hkx.objects().to_vec();
            let parent = behavior_parent_dir(&rel);
            for object in &objects {
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
            graph.files.push(Self::make(objects, rel));
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
            // space, entering at its graph `rootGenerator`. (§6a.3 may refine the entry point /
            // the inner directional-blend → leaf mapping.) Ancestors carry their own file ids,
            // so the upper (referring-file) links stay resolvable.
            if ref_depth >= 16 {
                return None; // cyclic / pathologically deep behaviorName chain
            }
            let name = string_member(o, "behaviorName")?;
            let tf = g.resolve(f, &name)?;
            let entry = g.files[tf].root_generator?;
            build(g, tf, entry, ancestors, ref_depth + 1, through_blender)
        }
        _ => {
            // Same generator-edge set as subtree_has_speed (the selection predicate): a stance
            // descends IN-FILE RifleRelaxed_SM → … → BSCyclicBlendTransitionGenerator
            // --pBlenderGenerator--> directional blend → loop clip. Without pBlenderGenerator/
            // pDefaultGenerator/layers, build() stalls before the locomotion leaves (§6a.3).
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

    fn rec(i: usize, anc: &[(usize, usize, usize, i64)], g: &BehaviorGraph) -> Entry {
        let (f, sm_idx, state_idx, _enter_ev) = anc[i];
        let sid = i64_member(&g.objs(f)[state_idx], "stateId").unwrap_or(0);
        let start = i64_member(&g.objs(f)[sm_idx], "startStateId").unwrap_or(0);
        let link = if i > 0 && sid != start {
            let (pf, psm, _pstate, penter) = anc[i - 1];
            let ev = if penter >= 0 {
                g.evs(pf).get(penter as usize).cloned().unwrap_or_default()
            } else {
                String::new()
            };
            let var =
                selector_var_of_sm(&g.objs(pf)[psm], g.objs(pf), g.vars(pf)).unwrap_or_default();
            Some((ev, var, Box::new(rec(i - 1, anc, g))))
        } else {
            None
        };
        Entry {
            state_id: sid,
            link,
        }
    }

    if anc.is_empty() {
        return Entry {
            state_id: 0,
            link: None,
        };
    }
    rec(anc.len() - 1, &anc, g)
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

/// The `.hkx` whose stem equals `leaf`, directly inside `dir` under `root` (non-recursive,
/// case-insensitive). This is the SAPT-override match: the clip resolved at the SAPT dir
/// itself rather than re-anchored at the base `Animations` root.
fn find_clip_stem_in_dir(root: &Path, dir: &str, leaf: &str) -> Option<PathBuf> {
    let disk_dir = resolve_case_insensitive(root, Path::new(dir))?;
    for e in std::fs::read_dir(disk_dir).ok()?.flatten() {
        let p = e.path();
        let is_hkx = p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| x.eq_ignore_ascii_case("hkx"));
        if p.is_file()
            && is_hkx
            && p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(leaf))
        {
            return Some(p);
        }
    }
    None
}

/// Resolve a speed clip's `animationName` to the on-disk `.hkx` along the subgraph SAPT
/// chain. `roots` is searched in order per SAPT dir (mod first, then base for weapons);
/// the SAPT chain is the override-priority outer loop. Creature callers pass `&[mod]`.
///
/// A **flat** clip name (one component under `Animations`, e.g. `Animations\RunForward.hkx`)
/// is redirected by the SAPT override dir, so it must resolve directly inside each SAPT dir —
/// the deepest/most-specific dir wins — BEFORE the declared-path rebuild. `declared_animation_path`
/// re-anchors the name at the base `Animations` root, which shadows an injured/override clip
/// (RE `grafton-animtext-three-emitter-defects`: the injured MegaSloth RunForward is a limp-run
/// ~3x slower than base). Sub-dir clip names — which carry their own path under `Animations`,
/// as weapon clips do — keep the declared-first order, so weapon resolution is unchanged.
fn clip_loop_path(clip: &HkxObject, roots: &[&Path], sapt_chain: &[String]) -> Option<PathBuf> {
    let animation_name = string_member(clip, "animationName")?;
    let leaf = leaf_basename(&animation_name);
    let comps: Vec<&str> = animation_name.split(['\\', '/']).collect();
    let flat = comps
        .iter()
        .position(|c| c.eq_ignore_ascii_case("Animations"))
        .is_some_and(|i| comps.len() - i - 1 == 1);
    for sapt in sapt_chain {
        let dir = sapt.trim_end_matches(['\r', '\n', ' ']).replace('\\', "/");
        for root in roots {
            if flat && let Some(p) = find_clip_stem_in_dir(root, &dir, &leaf) {
                return Some(p);
            }
            if let Some(relative) = declared_animation_path(&animation_name, sapt)
                && let Some(path) = resolve_case_insensitive(root, &relative)
                && path.is_file()
            {
                return Some(path);
            }
            if !flat && let Some(p) = find_clip_stem_in_dir(root, &dir, &leaf) {
                return Some(p);
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
    let dir_pos = leaf.ancestors.iter().rposition(|(f, sm_idx, _, _)| {
        selector_var_of_sm(&g.objs(*f)[*sm_idx], g.objs(*f), g.vars(*f)).as_deref()
            == Some("iSyncDirection")
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
/// container SM whose contour is invalid (Grafton: `RootBehavior` → `InvalidEntryLink`, so the
/// primary seed yields no body), fall through to the deep seed — the walk continued past the
/// dead-end to the unique speed-bearing child SM (`IdleLocomotion_SM`). The primary is always
/// tried first, so every creature whose default chain already reaches its locomotion SM is
/// byte-for-byte unchanged; the fallback can only turn a previously-empty output into a valid
/// one. A seed that exists but never yields an encodable contour is surfaced as a warning.
pub fn build_speed_info_body(
    core_behavior_disk: &Path,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    let g = BehaviorGraph::load_single(core_behavior_disk, roots)?;
    let graph_name = g.files[0].graph_name.clone();
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
    // A locomotion SM existed but no seed produced an encodable contour: previously a silent
    // `.ok()` dropped it without a trace. Surface it; the file stays absent so the engine
    // falls back to the Offsets locomotion loops. (Silent for genuine non-locomotion creatures,
    // which have no seed at all.)
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
// Weapon / character path: the locomotion contour lives in a base-game behavior REFERENCED
// by the (also base-game) core wrapping behavior, not the core itself — and its clips live
// in `extracted/fo4/Meshes`, not the mod. So the single-file, mod-only `build_speed_info_body`
// can't reach it. Rather than merge object spaces (which would mix per-file
// variableBindingSet/event index spaces and the wrong `hkbBehaviorGraph.name`), we run the
// EXISTING single-file contour on each reachable behavior — its own string data + graph name
// (`WeaponBehavior.hkb`) are then correct — and resolve loop clips across `[mod, base]`.
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
            graph_name: g.files[selection.file].graph_name.clone(),
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

fn cyclic_distance(value: f32, center: f32) -> f32 {
    let delta = (value - center).rem_euclid(std::f32::consts::TAU);
    delta.min(std::f32::consts::TAU - delta)
}

fn sampled_direction_source(
    g: &BehaviorGraph,
    file: usize,
    direction_state_machine: usize,
    state: usize,
    ordinal: usize,
    state_count: usize,
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
        let mut direction_angles = Vec::new();
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
            direction_angles
                .push(std::f32::consts::FRAC_PI_2 - std::f32::consts::TAU * direction_weight);
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
        let center = ordinal as f32 * std::f32::consts::TAU / state_count as f32;
        let center_mode = if cyclic_distance(center, 0.0) < 0.001 {
            CenterMode::ZeroCentered
        } else if cyclic_distance(center, std::f32::consts::PI) < 0.001 {
            CenterMode::PiCentered
        } else {
            continue;
        };
        for angle in &mut direction_angles {
            while *angle < center - std::f32::consts::PI {
                *angle += std::f32::consts::TAU;
            }
            while *angle > center + std::f32::consts::PI {
                *angle -= std::f32::consts::TAU;
            }
        }
        let direction_min = direction_angles.iter().copied().reduce(f32::min)?;
        let direction_max = direction_angles.iter().copied().reduce(f32::max)?;
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
    collect_objects_of_class(
        g.objs(file),
        root_state_machine,
        "hkbStateMachine",
        &mut HashSet::new(),
        &mut speed_state_machines,
    );
    for speed_state_machine in speed_state_machines {
        if selector_var_of_sm(
            &g.objs(file)[speed_state_machine],
            g.objs(file),
            g.vars(file),
        )
        .as_deref()
            != Some("iLocomotionSpeedState")
        {
            continue;
        }
        for speed_state in sm_states(&g.objs(file)[speed_state_machine], g.objs(file)).values() {
            let Some(generator) = first_ptr(&g.objs(file)[*speed_state], "generator") else {
                continue;
            };
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
                let directions: Option<Vec<_>> = states
                    .iter()
                    .enumerate()
                    .map(|(ordinal, state)| {
                        sampled_direction_source(
                            g,
                            file,
                            direction_state_machine,
                            *state,
                            ordinal,
                            states.len(),
                        )
                    })
                    .collect();
                let Some(mut directions) = directions else {
                    continue;
                };
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
    }
    None
}

fn find_state_path(
    objects: &[HkxObject],
    current: usize,
    target: usize,
    seen: &mut HashSet<usize>,
) -> Option<Vec<(usize, usize)>> {
    if current == target {
        return Some(Vec::new());
    }
    if !seen.insert(current) {
        return None;
    }
    let object = objects.get(current)?;
    if object.class_name == "hkbStateMachine" {
        for state in sm_states(object, objects).into_values() {
            let generator = first_ptr(&objects[state], "generator")?;
            if let Some(mut path) = find_state_path(objects, generator, target, seen) {
                path.insert(0, (current, state));
                seen.remove(&current);
                return Some(path);
            }
        }
    } else {
        for child in generator_children(object) {
            if let Some(path) = find_state_path(objects, child, target, seen) {
                seen.remove(&current);
                return Some(path);
            }
        }
    }
    seen.remove(&current);
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
    root_state_machine: usize,
    source: &SampledSource,
    direction: &SampledDirectionSource,
) -> Option<Vec<BehaviorReplay>> {
    let mut selections: Vec<_> = find_state_path(
        g.objs(source.file),
        root_state_machine,
        source.speed_state_machine,
        &mut HashSet::new(),
    )?
    .into_iter()
    .map(|(state_machine, state)| source_state_selection(source.file, state_machine, state))
    .collect();
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
    let sampled_collection = builder.record(
        ProducerClass::Collection,
        RecipeRecordParentage::Child {
            parent: *root_record,
        },
        source.file,
        source.direction_state_machine,
    )?;
    let mut sampled_children = Vec::with_capacity(source.directions.len());
    for direction in &source.directions {
        let record = builder.record(
            ProducerClass::SpeedSampled,
            RecipeRecordParentage::Child {
                parent: sampled_collection,
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
        let replay = sampled_replay(builder.graph, root_state_machine, source, direction).ok_or(
            SpeedInfoProducerError::MissingProducerData {
                behavior_file: source.file,
                object_index: source.speed_state_machine,
                field: "sampled source state path",
            },
        )?;
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
    children.insert(
        insertion.min(children.len()),
        RecipeContour::Collection {
            record: sampled_collection,
            children: sampled_children,
        },
    );
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
        let graph_name = g.files[f].graph_name.clone();
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
        if let Some(candidate) = locomotion_by_object.remove(&(file, state_machine)) {
            let resolved = resolve_weapon_node(&candidate.tree, &g, sapt_chain).ok_or(
                SpeedInfoProducerError::MissingProducerData {
                    behavior_file: file,
                    object_index: state_machine,
                    field: "resolved locomotion records",
                },
            )?;
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

struct EvaluationAssets {
    behaviors: HashMap<usize, HkxFile>,
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
        let bytes = std::fs::read(&disk)
            .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
        let hkx = read_packfile(&bytes)
            .map_err(|error| SpeedInfoProducerError::Evaluation(error.to_string()))?;
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
    let Some(first) = replay.first() else {
        return Err(SpeedInfoProducerError::Evaluation(
            "evaluation replay is empty".to_string(),
        ));
    };
    if replay.iter().any(|segment| segment.owner != first.owner) {
        return Err(SpeedInfoProducerError::Evaluation(
            "cross-behavior evaluation replay is unsupported".to_string(),
        ));
    }
    Ok((
        first.owner.behavior_file,
        LoadOptions {
            root: first.root.clone(),
            actions: replay
                .iter()
                .flat_map(|segment| segment.actions.iter().cloned())
                .collect(),
            root_motion_projection: RootMotionProjection::MagnitudeOnly,
        },
    ))
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
    if samples.len() <= 2 {
        return samples.to_vec();
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
    let direction_count =
        ((domain.direction_max - domain.direction_min) / domain.direction_step).trunc() as u32 + 1;
    let speed_count =
        ((domain.speed_max - domain.speed_min) / domain.speed_step).trunc() as u32 + 1;
    let mut curves = Vec::with_capacity(direction_count as usize);
    let mut direction = domain.direction_min;
    for _ in 0..direction_count {
        evaluator
            .set_variable(
                &domain.direction_variable,
                EvaluatorVariableValue::Real(direction),
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
        let samples = monotonic_speed_samples(&samples);
        let samples = chord_reduce_speed_samples(&samples, domain.chord_error_tolerance);
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

fn evaluate_recipe(
    recipe: &NeedsEvaluation,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Result<Vec<EvaluatedRequest>, SpeedInfoProducerError> {
    let assets = evaluation_assets(recipe, roots, sapt_chain)?;
    recipe
        .requests
        .iter()
        .enumerate()
        .map(|(request_id, request)| {
            let result = match request {
                EvaluationRequest::Individual(request) => {
                    let (speed, direction) =
                        value_dir(&request.animation_path).ok_or_else(|| {
                            SpeedInfoProducerError::Evaluation(format!(
                                "could not read root motion from {}",
                                request.animation_path.display()
                            ))
                        })?;
                    Ok(EvaluatedRequest::Individual { speed, direction })
                }
                EvaluationRequest::SpeedSampled(request) => {
                    evaluate_speed_sampled(request, &assets)
                }
                EvaluationRequest::RootMetadata(request) => {
                    evaluate_root_metadata(request, &assets)
                }
            };
            result.map_err(|error| {
                SpeedInfoProducerError::Evaluation(format!("request {request_id}: {error}"))
            })
        })
        .collect()
}

fn evaluated_request<'a>(
    evaluations: &'a [EvaluatedRequest],
    id: EvaluationRequestId,
) -> Result<&'a EvaluatedRequest, SpeedInfoProducerError> {
    evaluations
        .get(id.0 as usize)
        .ok_or(SpeedInfoProducerError::MissingEvaluation(id))
}

fn evaluated_entry(
    entry: &RecipeEntry,
    evaluations: &[EvaluatedRequest],
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
    evaluations: &[EvaluatedRequest],
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
    evaluations: &[EvaluatedRequest],
) -> Result<SpeedInfoFile, SpeedInfoProducerError> {
    let roots = recipe
        .roots
        .iter()
        .map(|root| {
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
        })
        .collect::<Result<Vec<_>, SpeedInfoProducerError>>()?;
    Ok(SpeedInfoFile { roots })
}

pub fn build_speed_info_body_weapon(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> Option<Vec<u8>> {
    let recipe = build_speed_info_recipe_weapon(core_rel, roots, sapt_chain).ok()?;
    let evaluations = evaluate_recipe(&recipe, roots, sapt_chain).ok()?;
    let speed_info = evaluated_speed_info(&recipe, &evaluations).ok()?;
    encode_speed_info(&speed_info).ok()
}

/// The loop-clip basenames (lowercased, no ext) owned by this weapon subgraph's
/// `AnimationSpeedInfo` contour — the cyclic directional locomotion leaves. `AnimationOffsets`
/// (§6b) must EXCLUDE these: the engine's locomotion system owns their root motion, so caching
/// them in Offsets too makes the subgraph moonwalk. Returns EMPTY for a subgraph with no
/// SpeedInfo contour (e.g. 1st-person `GunBehavior`) — then Offsets legitimately keeps its loops.
/// Same cross-file root selection as [`build_speed_info_body_weapon`] (kept in lockstep).
pub fn speed_info_leaf_basenames(
    core_rel: &str,
    roots: &[&Path],
    sapt_chain: &[String],
) -> BTreeSet<String> {
    let _ = sapt_chain; // the loop SET is SAPT-independent (selection is graph-only)
    let g = BehaviorGraph::load_reachable(core_rel, roots);
    let mut out: BTreeSet<String> = BTreeSet::new();
    for f in 0..g.files.len() {
        let objects = g.objs(f);
        if g.files[f].graph_name.is_empty() {
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
            if let Some(tree) = build(&g, f, idx, Vec::new(), 0, false) {
                collect_leaf_clips(&tree, &g, &mut out);
            }
        }
    }
    out
}

/// Walk a contour tree collecting each `Individual` leaf's loop-clip basename — but ONLY the
/// leaves bound to an `fLocomotion*PlaybackSpeed` variable (the cyclic directional walk/run/
/// sneak loops). The selection SMs also carry fire/reload clips bound to `weaponSpeedMult`/
/// `reloadSpeedMult`; those are NOT locomotion loops and must NOT be excluded from Offsets (CK
/// keeps `wpnreload`/`wpnfireauto*` in the offsets cache). Filtering on the bound `param` is what
/// makes this the precise moonwalk-guard set rather than "every speed-bound clip" (§6b.2).
fn collect_leaf_clips(node: &Node, g: &BehaviorGraph, out: &mut BTreeSet<String>) {
    match node {
        Node::Collection(ch) => {
            for c in ch {
                collect_leaf_clips(c, g, out);
            }
        }
        Node::Individual(leaf) => {
            let p = leaf.param.to_ascii_lowercase();
            if !(p.starts_with("flocomotion") && p.ends_with("playbackspeed")) {
                return;
            }
            let (cf, ci) = leaf.speed_clip;
            if let Some(an) = string_member(&g.objs(cf)[ci], "animationName") {
                out.insert(leaf_basename(&an));
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
