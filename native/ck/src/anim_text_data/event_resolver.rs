//! AnimEventInfo event→clip resolver: a behavior-graph state-machine walk (CK-free).
//!
//! The event→clip mapping is **NOT** ESP-derivable (RACE `ATKD` carries no anim field —
//! confirmed vs the FO4 schema + live DeathclawRace). The clip names live ONLY inside
//! the behavior graph, reached by a state-machine traversal. This module mirrors the
//! byte-exact-validated reference resolvers one-for-one:
//!   * `scratchpad/atd_re/{resolve_clips_v2,validate}.py` — Snallygaster 15/15, Floaters 19/19
//!   * `scratchpad/{walk5,flagcheck}.py` — deathclaw 26/28 (+flags 26/26), nested-SM recursion
//!
//! ## Emission model — one entry per *transition*, not per event
//!
//! A single ESP event can fire **several** transitions that resolve to **different**
//! clips with **different** flags, and CK emits each as its own line. E.g. deathclaw
//! `ThrowAttackStart` → `ThrowAttackMoving` (flag 1) **and** `ThrowAttackStart` →
//! `ThrowAttack` (flag 0); `evadeLeft` resolves twice. So we collect `(event, flag,
//! clips)` per transition and de-duplicate by `(event, clip-set)` (case-insensitive),
//! keeping the moving-gated flag on a tie.
//!
//! For each transition of every `hkbStateMachine` (`wildcardTransitions` + each state's
//! local `transitions`):
//! 1. **eventId** → event name via `hkbBehaviorGraphStringData.eventNames[]`. Emitted
//!    under the matching ESP candidate's spelling (CK's one case-insensitive table).
//! 2. **clips** — resolve the transition's target state to its clip name(s):
//!    - **nested SM** — if `flags & 0x2000` (`FLAG_TO_NESTED_STATE_ID_IS_VALID`), descend
//!      the target state's generator to the first nested `hkbStateMachine` (`find_sm`)
//!      and pick the state whose `stateId == toNestedStateId`, then resolve **its**
//!      generator. This is what selects the specific attack inside `AttackRoot_Behavior`
//!      (the side-swipes) — missing it drops the 2nd clip on multi-clip events.
//!    - otherwise resolve the target state's generator directly.
//!    Generator descent recurses `hkbModifierGenerator.generator`,
//!    `DynamicAnimationTaggingGenerator.pDefaultGenerator`,
//!    `hkbBlenderGenerator.children[]→hkbBlenderGeneratorChild.generator`,
//!    `hkbManualSelectorGenerator`/`hkbPoseMatchingGenerator.generators[]`+`children[]`,
//!    and **into nested `hkbStateMachine`s via `startStateId`**, to every reachable
//!    `hkbClipGenerator.name`.
//! 3. **flag** — `1` **iff** the triggering transition's `condition` is a *moving* speed
//!    gate (`Speed >= N` / `Speed > N …`); `0` otherwise. It is NOT a `transition.flags`
//!    bit (flag-0 and flag-1 oracle entries both carry `flags=8192`), NOT `clip.mode`,
//!    NOT `clip.flags`. Proven via the `ThrowAttackStart` minimal pair (`Speed >= 20` ⇒ 1,
//!    `Speed <= 20` ⇒ 0) — `scratchpad/flagcheck.py`, 26/26.
//! 4. **case substitution** — a resolved clip name that case-insensitively equals an ESP
//!    event name is emitted with the **event's** spelling (clip-gen `EvadeLeft` collides
//!    with event `evadeLeft` → prints `evadeLeft`).
//! 5. Drop any candidate that resolves to no clip (→ root behaviors emit the empty form).
//!
//! ## Known residual (honest gap)
//!
//! Two deathclaw oracle entries are **not** offline-derivable: `evadeLeft → DeathClaw
//! EvadeForwardMirrored` and `evadeRight → DeathClaw EvadeForward.HKT00`. The nested evade
//! SM contains only Left/Right (stateId 0/1) states; the *forward*-evade variant is chosen
//! at runtime by a movement-direction variable binding, not by the evade event — the same
//! runtime variable-index class as the deathclaw melee shared-selector. So the honest
//! ceiling on deathclaw is 26/28; the resolver reproduces those 26 with byte-exact clips
//! **and** flags, and emits no false positives. The remaining 2 require CK's runtime.
//!
//! NOTE: this resolver reads `states` / `transitions` / `generators` pointer arrays,
//! which require the TAG0 8-byte pointer-array stride; a 4-byte stride returns them
//! half-null and the walk silently yields 0 clips.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxMember, HkxObject};

use super::bucket_files::AnimEvent;

/// `FLAG_TO_NESTED_STATE_ID_IS_VALID` — when set on a transition, `toNestedStateId`
/// selects the state *inside* the target state's nested state machine.
const FLAG_TO_NESTED_STATE_ID_IS_VALID: i64 = 0x2000;

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

fn i64_in(members: &[HkxMember], name: &str) -> Option<i64> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| as_i64(&m.value))
}

fn member_value<'a>(obj: &'a HkxObject, name: &str) -> Option<&'a HkxValue> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .map(|m| &m.value)
}

fn ptr_member(obj: &HkxObject, name: &str) -> Option<usize> {
    obj.members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::Pointer(Some(i)) => Some(*i),
            _ => None,
        })
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

/// Object indices a generator-reference value points at (single pointer or array of
/// pointers). Inline value-objects are not used for generators in a packfile.
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

/// Object indices a named pointer/array member points at.
fn ptr_array(obj: &HkxObject, name: &str) -> Vec<usize> {
    member_value(obj, name).map(ptr_targets).unwrap_or_default()
}

fn first_ptr(obj: &HkxObject, name: &str) -> Option<usize> {
    ptr_array(obj, name).into_iter().next()
}

fn collect_event_names(objects: &[HkxObject]) -> Vec<String> {
    for obj in objects {
        if obj.class_name != "hkbBehaviorGraphStringData" {
            continue;
        }
        if let Some(items) = array_member(obj, "eventNames") {
            return items
                .iter()
                .map(|v| match v {
                    HkxValue::String { value, .. } => value.clone(),
                    _ => String::new(),
                })
                .collect();
        }
    }
    Vec::new()
}

/// `stateId -> object index` for one state machine's `states[]` (last wins, mirroring the
/// reference resolver's dict assignment).
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

// ---------------------------------------------------------------------------------------
// Generator descent — collect every reachable clip-generator name (class-explicit, the
// verified walk5.py dispatch). `seen` cycle-guards by object index.
// ---------------------------------------------------------------------------------------

fn recurse_ref(
    obj: &HkxObject,
    name: &str,
    objects: &[HkxObject],
    seen: &mut HashSet<usize>,
    out: &mut Vec<String>,
    depth: usize,
) {
    for t in ptr_array(obj, name) {
        clip_collect_idx(t, objects, seen, out, depth + 1);
    }
}

fn clip_collect_idx(
    i: usize,
    objects: &[HkxObject],
    seen: &mut HashSet<usize>,
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth > 60 || !seen.insert(i) {
        return;
    }
    let Some(o) = objects.get(i) else {
        return;
    };
    match o.class_name.as_str() {
        "hkbClipGenerator" => {
            if let Some(n) = string_member(o, "name") {
                out.push(n);
            }
        }
        "hkbModifierGenerator" => recurse_ref(o, "generator", objects, seen, out, depth),
        "DynamicAnimationTaggingGenerator" => {
            recurse_ref(o, "pDefaultGenerator", objects, seen, out, depth)
        }
        "hkbBlenderGenerator" => {
            for ch in ptr_array(o, "children") {
                if let Some(co) = objects.get(ch) {
                    if co.class_name == "hkbBlenderGeneratorChild" {
                        recurse_ref(co, "generator", objects, seen, out, depth);
                    }
                }
            }
        }
        "hkbManualSelectorGenerator" | "hkbPoseMatchingGenerator" => {
            for g in ptr_array(o, "generators") {
                clip_collect_idx(g, objects, seen, out, depth + 1);
            }
            for ch in ptr_array(o, "children") {
                if let Some(co) = objects.get(ch) {
                    if co.class_name == "hkbBlenderGeneratorChild" {
                        recurse_ref(co, "generator", objects, seen, out, depth);
                    }
                }
            }
        }
        "hkbStateMachine" => {
            // Nested SM (no toNestedStateId hint here): take the start state's generator.
            let states = sm_states(o, objects);
            let start = i64_member(o, "startStateId").unwrap_or(0);
            if let Some(&st) = states.get(&start) {
                if let Some(so) = objects.get(st) {
                    recurse_ref(so, "generator", objects, seen, out, depth);
                }
            }
        }
        _ => {}
    }
}

fn clips_under_generator(gen_val: &HkxValue, objects: &[HkxObject], out: &mut Vec<String>) {
    let mut seen = HashSet::new();
    for t in ptr_targets(gen_val) {
        clip_collect_idx(t, objects, &mut seen, out, 0);
    }
}

/// Descend a generator subtree to the **first nested `hkbStateMachine`** (mirrors
/// `walk5.py::find_sm`). Returns the SM's object index.
fn find_sm(i: usize, objects: &[HkxObject], depth: usize) -> Option<usize> {
    if depth > 20 {
        return None;
    }
    let o = objects.get(i)?;
    match o.class_name.as_str() {
        "hkbStateMachine" => Some(i),
        "hkbModifierGenerator" => {
            first_ptr(o, "generator").and_then(|j| find_sm(j, objects, depth + 1))
        }
        "hkbBlenderGenerator" => {
            for ch in ptr_array(o, "children") {
                if objects
                    .get(ch)
                    .is_some_and(|c| c.class_name == "hkbBlenderGeneratorChild")
                {
                    if let Some(g) = first_ptr(&objects[ch], "generator") {
                        if let Some(r) = find_sm(g, objects, depth + 1) {
                            return Some(r);
                        }
                    }
                }
            }
            None
        }
        "hkbManualSelectorGenerator" | "hkbPoseMatchingGenerator" => {
            for g in ptr_array(o, "generators") {
                if let Some(r) = find_sm(g, objects, depth + 1) {
                    return Some(r);
                }
            }
            None
        }
        _ => None,
    }
}

/// Resolve one transition's target state to its clip name(s). When `flags` carries
/// `FLAG_TO_NESTED_STATE_ID_IS_VALID`, descend to the nested SM and pick the state whose
/// `stateId == to_nested`; otherwise resolve the target state's generator directly.
fn resolve_transition(
    tgt_idx: usize,
    to_nested: i64,
    flags: i64,
    objects: &[HkxObject],
) -> BTreeSet<String> {
    let gen_val = objects
        .get(tgt_idx)
        .and_then(|o| member_value(o, "generator"));

    if flags & FLAG_TO_NESTED_STATE_ID_IS_VALID != 0 {
        if let Some(gv) = gen_val {
            if let Some(sm_idx) = ptr_targets(gv)
                .first()
                .and_then(|&j| find_sm(j, objects, 0))
            {
                let states = sm_states(&objects[sm_idx], objects);
                if let Some(&ns) = states.get(&to_nested) {
                    let mut out = Vec::new();
                    if let Some(nv) = objects.get(ns).and_then(|o| member_value(o, "generator")) {
                        clips_under_generator(nv, objects, &mut out);
                    }
                    return out.into_iter().filter(|c| !c.is_empty()).collect();
                }
            }
        }
    }

    let mut out = Vec::new();
    if let Some(gv) = gen_val {
        clips_under_generator(gv, objects, &mut out);
    }
    out.into_iter().filter(|c| !c.is_empty()).collect()
}

// ---------------------------------------------------------------------------------------
// Transitions + flag (moving speed-gate condition)
// ---------------------------------------------------------------------------------------

struct Trans {
    eid: i64,
    tsid: i64,
    flags: i64,
    to_nested: i64,
    cond: Option<String>,
}

/// The transition's `condition` expression text, if any (`hkbExpressionCondition.expression`
/// or `hkbStringCondition.conditionString`).
fn cond_expr(members: &[HkxMember], objects: &[HkxObject]) -> Option<String> {
    let cv = members
        .iter()
        .find(|m| m.name == "condition")
        .map(|m| &m.value)?;
    let idx = match cv {
        HkxValue::Pointer(Some(i)) => *i,
        _ => return None,
    };
    let o = objects.get(idx)?;
    string_member(o, "expression").or_else(|| string_member(o, "conditionString"))
}

/// A *moving* gate references `Speed` with a `>` / `>=` lower bound (mirrors
/// `flagcheck.py::is_moving`). `Speed <= N`, `Speed == N`, non-speed conditions ⇒ not moving.
fn is_moving_condition(expr: Option<&str>) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"Speed\s*>=?\s*\d").unwrap());
    expr.is_some_and(|e| re.is_match(e))
}

fn read_transitions(ta_idx: usize, objects: &[HkxObject]) -> Vec<Trans> {
    let Some(arr) = objects.get(ta_idx) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(items) = array_member(arr, "transitions") {
        for it in items {
            let members = match it {
                HkxValue::Pointer(Some(i)) => match objects.get(*i) {
                    Some(o) => o.members.as_slice(),
                    None => continue,
                },
                _ => match it.as_object_members() {
                    Some(ms) => ms,
                    None => continue,
                },
            };
            let (Some(eid), Some(tsid)) =
                (i64_in(members, "eventId"), i64_in(members, "toStateId"))
            else {
                continue;
            };
            out.push(Trans {
                eid,
                tsid,
                flags: i64_in(members, "flags").unwrap_or(0),
                to_nested: i64_in(members, "toNestedStateId").unwrap_or(0),
                cond: cond_expr(members, objects),
            });
        }
    }
    out
}

/// Every `(eventId, flag, clip-set)` entry the graph yields, one per resolving transition.
fn collect_entries(objects: &[HkxObject]) -> Vec<(i64, u32, BTreeSet<String>)> {
    let mut out = Vec::new();
    for sm in objects.iter().filter(|o| o.class_name == "hkbStateMachine") {
        let states = sm_states(sm, objects);
        let mut tas: Vec<usize> = Vec::new();
        if let Some(wc) = ptr_member(sm, "wildcardTransitions") {
            tas.push(wc);
        }
        for &st in states.values() {
            if let Some(lt) = objects.get(st).and_then(|o| ptr_member(o, "transitions")) {
                tas.push(lt);
            }
        }
        for ta in tas {
            for t in read_transitions(ta, objects) {
                let Some(&tgt) = states.get(&t.tsid) else {
                    continue;
                };
                let clips = resolve_transition(tgt, t.to_nested, t.flags, objects);
                if clips.is_empty() {
                    continue;
                }
                let flag = u32::from(is_moving_condition(t.cond.as_deref()));
                out.push((t.eid, flag, clips));
            }
        }
    }
    out
}

/// Resolve the `AnimEventInfo` entries for a behavior, given the ESP candidate event set
/// (`candidates` = RACE `ATKE` + whitelisted IDLE `ENAM`, original spelling).
///
/// Per resolving transition: look up the event name, keep it only if it matches a candidate
/// (case-insensitive), substitute the candidate's spelling, case-substitute the clip names,
/// and emit `(event, flag, clips)`. De-duplicated by `(event, clip-set)` — keeping the
/// moving-gated `flag=1` on a tie. Returns empty when nothing resolves (a wrong
/// AnimEventInfo is worse than none — caller emits no file).
pub fn resolve_anim_events(behavior_file: &Path, candidates: &[String]) -> Vec<AnimEvent> {
    let Ok(data) = std::fs::read(behavior_file) else {
        return Vec::new();
    };
    let Ok(hkx) = read_packfile(&data) else {
        return Vec::new();
    };
    let objects = hkx.objects();

    let event_names = collect_event_names(objects);
    let entries = collect_entries(objects);

    // ESP candidate set: ci map (first spelling wins) for case-substitution + filtering.
    let mut ev_by_lower: HashMap<String, String> = HashMap::new();
    for c in candidates {
        ev_by_lower
            .entry(c.to_ascii_lowercase())
            .or_insert_with(|| c.clone());
    }

    // De-dup by (event_lc, clip-set_lc); prefer the moving-gated flag.
    let mut dedup: BTreeMap<(String, Vec<String>), AnimEvent> = BTreeMap::new();
    for (eid, flag, clips) in entries {
        if eid < 0 {
            continue;
        }
        let Some(ev_name) = event_names.get(eid as usize) else {
            continue;
        };
        let lc = ev_name.to_ascii_lowercase();
        let Some(spelling) = ev_by_lower.get(&lc) else {
            continue; // not an ESP candidate event
        };
        let subst: Vec<String> = clips
            .iter()
            .map(|c| {
                ev_by_lower
                    .get(&c.to_ascii_lowercase())
                    .cloned()
                    .unwrap_or_else(|| c.clone())
            })
            .collect();
        let key = (
            lc.clone(),
            subst.iter().map(|s| s.to_ascii_lowercase()).collect(),
        );
        dedup
            .entry(key)
            .and_modify(|e| {
                if e.flag == 0 && flag == 1 {
                    e.flag = 1;
                }
            })
            .or_insert_with(|| AnimEvent {
                name: spelling.clone(),
                flag,
                clips: subst,
            });
    }

    let mut out: Vec<AnimEvent> = dedup.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.clips.cmp(&b.clips)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn extracted_meshes() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/fo4/meshes")
    }

    /// `(event, flag, clips)` rows from a CK AnimEventInfo oracle (repeats preserved).
    fn oracle_rows(text: &str) -> Vec<(String, u32, Vec<String>)> {
        let mut lines = text.split('\n');
        assert_eq!(lines.next().unwrap(), "V2");
        let _path = lines.next().unwrap();
        assert_eq!(lines.next().unwrap(), "");
        let count: usize = lines.next().unwrap().trim().parse().unwrap();
        let mut out = Vec::new();
        for _ in 0..count {
            let name = lines.next().unwrap().to_string();
            let flag: u32 = lines.next().unwrap().trim().parse().unwrap();
            let nclips: usize = lines.next().unwrap().trim().parse().unwrap();
            let clips: Vec<String> = (0..nclips)
                .map(|_| lines.next().unwrap().to_string())
                .collect();
            out.push((name, flag, clips));
        }
        out
    }

    /// Case-insensitive `(event, sorted clip-set)` key.
    fn key_of(name: &str, clips: &[String]) -> (String, Vec<String>) {
        let mut cl: Vec<String> = clips.iter().map(|c| c.to_ascii_lowercase()).collect();
        cl.sort();
        (name.to_ascii_lowercase(), cl)
    }

    /// Distinct candidate event spellings (first wins), the ESP-side input.
    fn distinct_candidates(rows: &[(String, u32, Vec<String>)]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (n, _, _) in rows {
            if seen.insert(n.to_ascii_lowercase()) {
                out.push(n.clone());
            }
        }
        out
    }

    /// Deathclaw (vanilla FO4): byte-exact clips **and** condition-derived flags on every
    /// offline-derivable entry, with **no false positives**. The only two oracle entries
    /// the resolver cannot reach are the runtime movement-direction forward-evade variants
    /// (documented in the module header) — pinned here so a regression in either direction
    /// (losing a derivable entry, or fabricating a forward-evade) fails the test.
    #[test]
    fn resolver_deathclaw_nested_sm_and_flag_byte_exact_except_runtime_forward_evade() {
        let meshes = extracted_meshes();
        let behavior = meshes.join("actors/deathclaw/behaviors/deathclaweverything.hkx");
        let oracle_path = meshes.join("animtextdata/animeventinfo/1427226682.txt");
        if !behavior.is_file() || !oracle_path.is_file() {
            eprintln!("extracted/fo4 deathclaw absent; skipping");
            return;
        }
        let rows = oracle_rows(&std::fs::read_to_string(&oracle_path).unwrap());
        let candidates = distinct_candidates(&rows);
        let mine = resolve_anim_events(&behavior, &candidates);

        let oracle: BTreeMap<(String, Vec<String>), u32> =
            rows.iter().map(|(n, f, c)| (key_of(n, c), *f)).collect();
        let ours: BTreeMap<(String, Vec<String>), u32> = mine
            .iter()
            .map(|e| (key_of(&e.name, &e.clips), e.flag))
            .collect();

        // No false positives — every emitted entry is in the oracle with the SAME flag+clips.
        for (k, f) in &ours {
            assert_eq!(
                oracle.get(k),
                Some(f),
                "deathclaw entry {k:?}: clip/flag mismatch or not in oracle (false positive)"
            );
        }

        // The two runtime-only forward-evade entries (selected by a movement-direction
        // variable, not the evade event) are the sole permitted misses.
        let forward_mirrored = key_of("evadeLeft", &["DeathClaw EvadeForwardMirrored".to_string()]);
        let forward_hkt00 = key_of("evadeRight", &["DeathClaw EvadeForward.HKT00".to_string()]);
        let missing: BTreeSet<(String, Vec<String>)> = oracle
            .keys()
            .filter(|k| !ours.contains_key(*k))
            .cloned()
            .collect();
        let expected_missing: BTreeSet<(String, Vec<String>)> =
            [forward_mirrored, forward_hkt00].into_iter().collect();
        assert_eq!(
            missing, expected_missing,
            "deathclaw misses are not exactly the two runtime forward-evade variants"
        );

        // The proven counts: 28 oracle entries, 26 reproduced byte-exact.
        assert_eq!(oracle.len(), 28, "oracle entry count drifted");
        assert_eq!(ours.len(), 26, "resolver should reproduce exactly 26 of 28");

        // Load-bearing spot checks the walk MUST recover (unsynthesizable from any ESP
        // string): nested-SM side-swipe with the moving-gate flag, and the flag split.
        assert_eq!(
            ours.get(&key_of(
                "meleeAttackStartLeftSideSwipeStart",
                &["DeathclawAttackLeftSideSwipe".to_string()]
            )),
            Some(&1),
            "nested-SM side-swipe must resolve with flag=1 (Speed > 20)"
        );
        assert_eq!(
            ours.get(&key_of(
                "ThrowAttackStart",
                &["ThrowAttackMoving".to_string()]
            )),
            Some(&1),
            "ThrowAttackStart moving variant must be flag=1"
        );
        assert_eq!(
            ours.get(&key_of("ThrowAttackStart", &["ThrowAttack".to_string()])),
            Some(&0),
            "ThrowAttackStart standing variant must be flag=0"
        );
    }

}
