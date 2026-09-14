//! Give every attack clip a `HitFrame` trigger, because FO4 accepts no substitute.
//!
//! FO4 derives an attack's `attackTime` from exactly one event name. `CalculateAnimData`
//! (`0xF7CE20`) zeroes the slot, calls `0x6FF0F0`, and writes the result back:
//!
//! ```text
//!   0x140f7d380  mov   dword ptr [rbp+0x88], 0    ; attackTime = 0
//!   0x140f7d38a  call  0x1406ff0f0
//!   0x140f7d3e6  movss dword ptr [rcx+0x24], xmm0 ; attackTime
//!   0x140f7d3eb  movss dword ptr [rcx+0x20], xmm1 ; attackDistance
//! ```
//!
//! `0x6FF0F0` looks up the interned string held in the global at RVA `0x30E1A40`
//! (`"HitFrame"`, literal at RVA `0x24738F0`) and on a miss returns false without writing
//! either output, so a clip with no `HitFrame` yields `attackTime == 0` and
//! `attackDistance == 0`. Combat's first filter drops candidates with `attackTime <= 0`, so
//! such a creature never swings.
//!
//! FO76 marks the hit three ways:
//! 1. `HitFrame` on the clip generator's trigger array (RadHog). Works in FO4: RadHog has
//!    zero `HitFrame` in all 51 of its animation files and melees fine.
//! 2. `HitFrame` in the animation's own annotation track (Snallygaster 11 of 54 files,
//!    Megasloth 8 of 51). Works in FO4; Megasloth's `Attack1`..`Attack4` carry only
//!    `ReturnToDefaultFast` on the trigger side.
//! 3. A damage window instead of an instant: `WeaponSweepAttackStart` /
//!    `WeaponSweepAttackStop`, or `AreaAttackStart` for area attacks, with no `HitFrame`
//!    anywhere. FO4 has no such path.
//!
//! Across every FO76 actor behavior, 36 attack clips open a damage window and none also fires
//! `HitFrame`. They belong to Sheepsquatch (14), JerseyDevil (9), Trogg (7) and ScorchBeast
//! (6), none of which carry `HitFrame` on the animation side. In-game, all six ScorchBeast
//! attack clips pass every gate and still report `time=0.000 dist=0`.
//!
//! This pass adds `HitFrame` where the FO76 damage window opens, copying the donor trigger's
//! flags so the timing basis (`relativeToEndOfClip`) cannot drift. It repairs the behavior
//! graph rather than the emitted bucket because the graph feeds both: `ClipGeneratorData`
//! gives combat its `attackTime`, and the graph must fire `HitFrame` during playback for the
//! hit to land.
//!
//! Guards against a double or invented hit:
//! * a clip whose animation already carries a `HitFrame` annotation (convention 2) is skipped;
//! * a clip with no damage window gets nothing; there is no defensible time to guess.
//!
//! Scoped to the AnimEventInfo clip targets: a non-combat clip that sweeps is not an attack.
//!
//! # FixupReport mapping
//! `records_changed` = number of clip generators given a `HitFrame` trigger.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use havok_native::hkx::read_packfile;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxMember, HkxObject};

use super::behavior_index::{clip_leaf, core_project_dir, resolve_leaf};
use super::emit::{AnimTextDataInputs, SubgraphInput};
use super::event_resolver::resolve_anim_events;

const HIT_FRAME: &str = "HitFrame";

/// FO76's damage-window openers, in the order we prefer them. `WeaponSweepAttackStart` is
/// the melee sweep; `AreaAttackStart` covers area attacks like ScorchBeast's ground EMP,
/// which has no sweep at all.
const DONOR_EVENTS: [&str; 2] = ["WeaponSweepAttackStart", "AreaAttackStart"];

/// A clip generator that needs a `HitFrame`, and the trigger to model it on.
struct Missing {
    /// Index of the clip's `hkbClipTriggerArray` in the file's object table.
    trigger_array: usize,
    local_time: f32,
    relative_to_end_of_clip: HkxValue,
    acyclic: HkxValue,
    is_annotation: HkxValue,
}

pub fn synthesize_missing_hit_frames(
    inputs: &AnimTextDataInputs,
    src_meshes_root: &Path,
) -> Result<u32, String> {
    if inputs.event_candidates.is_empty() {
        return Ok(0);
    }

    let mut added = 0u32;
    let mut seen_core: BTreeSet<String> = BTreeSet::new();
    for subgraph in &inputs.subgraphs {
        if !seen_core.insert(subgraph.core_behavior.to_ascii_lowercase()) {
            continue;
        }
        let core_file = src_meshes_root.join(subgraph.core_behavior.replace('\\', "/"));
        if !core_file.is_file() {
            continue;
        }
        added += repair_one_core(
            &core_file,
            subgraph,
            src_meshes_root,
            &inputs.event_candidates,
        )?;
    }
    if added > 0 {
        // Same invalidation the alignment pass needs, for the same reason: deciding what to
        // repair reads through `hkx_cache::behavior_packfile`, so the memo now holds the
        // PRE-repair parse of every graph touched. Leave it and the buckets emit the old
        // trigger lists while the graph on disk is correct.
        super::hkx_cache::clear_all();
    }
    Ok(added)
}

fn repair_one_core(
    core_file: &Path,
    subgraph: &SubgraphInput,
    src_meshes_root: &Path,
    event_candidates: &[String],
) -> Result<u32, String> {
    let events = resolve_anim_events(core_file, event_candidates);
    if events.is_empty() {
        return Ok(0);
    }
    let combat_clips: BTreeSet<String> = events
        .iter()
        .flat_map(|e| e.clips.iter().map(|c| c.to_ascii_lowercase()))
        .collect();

    let data = std::fs::read(core_file).map_err(|e| format!("{}: {e}", core_file.display()))?;
    let mut hkx = read_packfile(&data).map_err(|e| format!("{}: {e}", core_file.display()))?;

    let names = event_names(hkx.objects());
    if names.is_empty() {
        return Ok(0);
    }
    let by_index: BTreeMap<usize, &str> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (i, n.as_str()))
        .collect();

    // Survey before mutating: `objects_mut` hands out one borrow at a time, and the event
    // id may not exist yet.
    let core_project = core_project_dir(src_meshes_root, core_file);
    let missing = collect_missing(
        hkx.objects(),
        &combat_clips,
        &by_index,
        src_meshes_root,
        subgraph,
        core_project.as_deref(),
    );
    if missing.is_empty() {
        return Ok(0);
    }

    let hit_id = match names.iter().position(|n| n.eq_ignore_ascii_case(HIT_FRAME)) {
        Some(index) => index,
        None => append_event(&mut hkx, names.len())?,
    };

    let mut added = 0u32;
    for entry in &missing {
        let object = &mut hkx.objects_mut()[entry.trigger_array];
        let Some(member) = object.members.iter_mut().find(|m| m.name == "triggers") else {
            continue;
        };
        let HkxValue::Array(list) = &mut member.value else {
            continue;
        };
        list.push(new_trigger(entry, hit_id));
        added += 1;
    }

    if added > 0 {
        let out = hkx.save();
        std::fs::write(core_file, out).map_err(|e| format!("{}: {e}", core_file.display()))?;
        register_in_chain(core_file)?;
    }
    Ok(added)
}

/// Teach the rest of this actor's attack chain the `HitFrame` name.
///
/// The core behavior is mounted into a root graph, and Havok maps a nested graph's events
/// up to its parent by name; an event the parent lacks dies inside the child. In-game,
/// `ScorchBeastCore.hkx` raised `preHitFrame`, `weaponSwing`, `WeaponSweepAttackStart` and
/// `WeaponSweepAttackStop` but never `HitFrame`, because `ScorchBeast.hkx` (175 events) had
/// all of those and not `HitFrame`. Fixing only the clip-owning graph fixes `attackTime`
/// but lands no damage.
///
/// Siblings are chosen by donor event, not filename: one that knows
/// `WeaponSweepAttackStart` is part of this attack chain, which finds the root without a
/// naming convention and never reaches the shared `Actors\Character\Behaviors\*` graphs.
/// Registration only; no triggers are added.
fn register_in_chain(core_file: &Path) -> Result<u32, String> {
    let Some(dir) = core_file.parent() else {
        return Ok(0);
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };

    let mut registered = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path == core_file
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("hkx"))
        {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let Ok(mut hkx) = read_packfile(&data) else {
            continue;
        };
        let names = event_names(hkx.objects());
        if names.is_empty() || names.iter().any(|n| n.eq_ignore_ascii_case(HIT_FRAME)) {
            continue;
        }
        if !names
            .iter()
            .any(|n| DONOR_EVENTS.iter().any(|d| d.eq_ignore_ascii_case(n)))
        {
            continue; // not part of this attack chain
        }
        if append_event(&mut hkx, names.len()).is_ok() {
            let out = hkx.save();
            std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
            registered += 1;
        }
    }
    Ok(registered)
}

fn collect_missing(
    objects: &[HkxObject],
    combat_clips: &BTreeSet<String>,
    names: &BTreeMap<usize, &str>,
    src_meshes_root: &Path,
    subgraph: &SubgraphInput,
    core_project: Option<&str>,
) -> Vec<Missing> {
    let mut out = Vec::new();
    for obj in objects {
        if obj.class_name != "hkbClipGenerator" {
            continue;
        }
        let Some(name) = string_member(&obj.members, "name") else {
            continue;
        };
        if !combat_clips.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        if animation_fires_hit_frame(&obj.members, src_meshes_root, subgraph, core_project) {
            continue; // convention 2 — the animation already lands the hit
        }
        let Some(array_index) = pointer_member(&obj.members, "triggers") else {
            continue; // no trigger array at all — nothing to model a hit on
        };
        let Some(array) = objects.get(array_index) else {
            continue;
        };
        let Some(triggers) = array_member(&array.members, "triggers") else {
            continue;
        };

        let mut donor: Option<(usize, &Vec<HkxMember>)> = None;
        let mut already_has_hit = false;
        for trigger in triggers {
            let members = match trigger {
                HkxValue::Object(m) => m,
                HkxValue::TypedObject { members, .. } => members,
                _ => continue,
            };
            let Some(event_name) = event_name_of(members, names) else {
                continue;
            };
            if event_name.eq_ignore_ascii_case(HIT_FRAME) {
                already_has_hit = true;
                break;
            }
            // Prefer the earliest-listed donor by DONOR_EVENTS rank, so a clip carrying both
            // a sweep and an area marker resolves deterministically.
            if let Some(rank) = DONOR_EVENTS
                .iter()
                .position(|d| d.eq_ignore_ascii_case(event_name))
            {
                if donor.as_ref().is_none_or(|(best, _)| rank < *best) {
                    donor = Some((rank, members));
                }
            }
        }
        if already_has_hit {
            continue;
        }
        let Some((_, members)) = donor else {
            continue; // no damage window to derive a hit from; do not invent one
        };
        let Some(local_time) = f32_member(members, "localTime") else {
            continue;
        };
        out.push(Missing {
            trigger_array: array_index,
            local_time,
            relative_to_end_of_clip: flag_of(members, "relativeToEndOfClip"),
            acyclic: flag_of(members, "acyclic"),
            is_annotation: flag_of(members, "isAnnotation"),
        });
    }
    out
}

/// Does this clip's animation already carry a `HitFrame` annotation?
///
/// Byte scan rather than a full parse: the annotation name lives NUL-delimited in the
/// animation's string table, and `\0HitFrame\0` distinguishes it from `preHitFrame`, which
/// these clips also carry. An animation not found on disk counts as not firing, leaving the
/// trigger side as the only source.
fn animation_fires_hit_frame(
    members: &[HkxMember],
    src_meshes_root: &Path,
    subgraph: &SubgraphInput,
    core_project: Option<&str>,
) -> bool {
    let Some(animation) = string_member(members, "animationName") else {
        return false;
    };
    if animation.is_empty() {
        return false;
    }
    let leaf = clip_leaf(&animation);
    let rel = resolve_leaf(src_meshes_root, &subgraph.sapt_chain, &leaf, core_project);
    let Ok(data) = std::fs::read(src_meshes_root.join(rel.replace('\\', "/"))) else {
        return false;
    };
    bytes_fire_hit_frame(&data)
}

fn bytes_fire_hit_frame(data: &[u8]) -> bool {
    let needle = b"\0HitFrame\0";
    data.windows(needle.len()).any(|w| w == needle)
}

/// Append `HitFrame` to `eventNames` and a matching default entry to `eventInfos`.
/// The two arrays are indexed in lockstep by every `hkbEventProperty::id`, so a push to
/// one without the other silently shifts every event id past the end of the shorter array.
fn append_event(
    hkx: &mut havok_native::hkx::HkxFile,
    expected_len: usize,
) -> Result<usize, String> {
    let mut names_pushed = false;
    let mut infos_pushed = false;

    for obj in hkx.objects_mut() {
        match obj.class_name.as_str() {
            "hkbBehaviorGraphStringData" => {
                if let Some(member) = obj.members.iter_mut().find(|m| m.name == "eventNames") {
                    if let HkxValue::Array(list) = &mut member.value {
                        if list.len() == expected_len {
                            list.push(HkxValue::String {
                                value: HIT_FRAME.to_string(),
                                is_null: false,
                            });
                            names_pushed = true;
                        }
                    }
                }
            }
            "hkbBehaviorGraphData" => {
                if let Some(member) = obj.members.iter_mut().find(|m| m.name == "eventInfos") {
                    if let HkxValue::Array(list) = &mut member.value {
                        if list.len() == expected_len {
                            list.push(HkxValue::Object(vec![HkxMember {
                                name: "flags".to_string(),
                                value: HkxValue::I8(0),
                            }]));
                            infos_pushed = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if !names_pushed || !infos_pushed {
        return Err(format!(
            "cannot register HitFrame: eventNames pushed={names_pushed} \
             eventInfos pushed={infos_pushed} (expected both at len {expected_len})"
        ));
    }
    Ok(expected_len)
}

fn new_trigger(entry: &Missing, event_id: usize) -> HkxValue {
    HkxValue::Object(vec![
        HkxMember {
            name: "localTime".to_string(),
            value: HkxValue::F32(entry.local_time),
        },
        HkxMember {
            name: "event".to_string(),
            value: HkxValue::Object(vec![
                HkxMember {
                    name: "id".to_string(),
                    value: HkxValue::I32(event_id as i32),
                },
                HkxMember {
                    name: "payload".to_string(),
                    value: HkxValue::Pointer(None),
                },
            ]),
        },
        HkxMember {
            name: "relativeToEndOfClip".to_string(),
            value: entry.relative_to_end_of_clip.clone(),
        },
        HkxMember {
            name: "acyclic".to_string(),
            value: entry.acyclic.clone(),
        },
        HkxMember {
            name: "isAnnotation".to_string(),
            value: entry.is_annotation.clone(),
        },
    ])
}

fn event_names(objects: &[HkxObject]) -> Vec<String> {
    for obj in objects {
        if obj.class_name != "hkbBehaviorGraphStringData" {
            continue;
        }
        let Some(list) = array_member(&obj.members, "eventNames") else {
            continue;
        };
        return list
            .iter()
            .map(|v| match v {
                HkxValue::String { value, .. } => value.clone(),
                _ => String::new(),
            })
            .collect();
    }
    Vec::new()
}

fn event_name_of<'a>(members: &[HkxMember], names: &BTreeMap<usize, &'a str>) -> Option<&'a str> {
    let event = members.iter().find(|m| m.name == "event")?;
    let inner = match &event.value {
        HkxValue::Object(m) => m,
        HkxValue::TypedObject { members, .. } => members,
        _ => return None,
    };
    let id = int_member(inner, "id")?;
    names.get(&id).copied()
}

fn flag_of(members: &[HkxMember], name: &str) -> HkxValue {
    members
        .iter()
        .find(|m| m.name == name)
        .map(|m| m.value.clone())
        .unwrap_or(HkxValue::Bool(false))
}

fn string_member(members: &[HkxMember], name: &str) -> Option<String> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::String { value, .. } => Some(value.clone()),
            _ => None,
        })
}

fn array_member<'a>(members: &'a [HkxMember], name: &str) -> Option<&'a Vec<HkxValue>> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::Array(list) => Some(list),
            _ => None,
        })
}

fn pointer_member(members: &[HkxMember], name: &str) -> Option<usize> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::Pointer(target) => *target,
            _ => None,
        })
}

fn f32_member(members: &[HkxMember], name: &str) -> Option<f32> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::F32(v) => Some(*v),
            HkxValue::Half(v) => Some(*v),
            _ => None,
        })
}

fn int_member(members: &[HkxMember], name: &str) -> Option<usize> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| match &m.value {
            HkxValue::I8(v) => usize::try_from(*v).ok(),
            HkxValue::U8(v) => Some(*v as usize),
            HkxValue::I16(v) => usize::try_from(*v).ok(),
            HkxValue::U16(v) => Some(*v as usize),
            HkxValue::I32(v) => usize::try_from(*v).ok(),
            HkxValue::U32(v) => usize::try_from(*v).ok(),
            HkxValue::I64(v) => usize::try_from(*v).ok(),
            HkxValue::U64(v) => usize::try_from(*v).ok(),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(event_id: i32, time: f32) -> HkxValue {
        HkxValue::Object(vec![
            HkxMember {
                name: "localTime".into(),
                value: HkxValue::F32(time),
            },
            HkxMember {
                name: "event".into(),
                value: HkxValue::Object(vec![
                    HkxMember {
                        name: "id".into(),
                        value: HkxValue::I32(event_id),
                    },
                    HkxMember {
                        name: "payload".into(),
                        value: HkxValue::Pointer(None),
                    },
                ]),
            },
            HkxMember {
                name: "relativeToEndOfClip".into(),
                value: HkxValue::Bool(false),
            },
            HkxMember {
                name: "acyclic".into(),
                value: HkxValue::Bool(false),
            },
            HkxMember {
                name: "isAnnotation".into(),
                value: HkxValue::Bool(false),
            },
        ])
    }

    fn clip(name: &str, triggers_index: usize) -> HkxObject {
        HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hkbClipGenerator".into(),
            members: vec![
                HkxMember {
                    name: "name".into(),
                    value: HkxValue::String {
                        value: name.into(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "triggers".into(),
                    value: HkxValue::Pointer(Some(triggers_index)),
                },
            ],
        }
    }

    fn trigger_array(triggers: Vec<HkxValue>) -> HkxObject {
        HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hkbClipTriggerArray".into(),
            members: vec![HkxMember {
                name: "triggers".into(),
                value: HkxValue::Array(triggers),
            }],
        }
    }

    fn names_map(names: &[&'static str]) -> BTreeMap<usize, &'static str> {
        names.iter().enumerate().map(|(i, n)| (i, *n)).collect()
    }

    /// The clips built here carry no `animationName`, so the animation-side guard resolves
    /// nothing and the trigger array is the only source under test.
    fn survey(
        objects: &[HkxObject],
        combat: &BTreeSet<String>,
        names: &BTreeMap<usize, &str>,
    ) -> Vec<Missing> {
        let subgraph = SubgraphInput {
            core_behavior: r"Actors\X\Behaviors\XCoreBehavior.hkx".into(),
            sapt_chain: vec![r"Actors\X\Animations".into()],
            race_dir: None,
        };
        collect_missing(objects, combat, names, Path::new("."), &subgraph, None)
    }

    #[test]
    fn a_sweep_window_supplies_the_hit_time() {
        let names = names_map(&["WeaponSweepAttackStart", "WeaponSweepAttackStop"]);
        let objects = vec![
            clip("WingSwipeLeft", 1),
            trigger_array(vec![trigger(0, 1.2), trigger(1, 1.4)]),
        ];
        let combat = BTreeSet::from(["wingswipeleft".to_string()]);

        let missing = survey(&objects, &combat, &names);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].local_time, 1.2);
        assert_eq!(missing[0].trigger_array, 1);
    }

    #[test]
    fn a_clip_that_already_hits_is_left_alone() {
        let names = names_map(&["WeaponSweepAttackStart", "HitFrame"]);
        let objects = vec![
            clip("WingSwipeLeft", 1),
            trigger_array(vec![trigger(0, 1.2), trigger(1, 1.3)]),
        ];
        let combat = BTreeSet::from(["wingswipeleft".to_string()]);

        assert!(survey(&objects, &combat, &names).is_empty());
    }

    #[test]
    fn area_attacks_are_covered_when_there_is_no_sweep() {
        let names = names_map(&["AreaAttackStart", "CameraShake"]);
        let objects = vec![
            clip("GroundAreaAttackClip", 1),
            trigger_array(vec![trigger(1, 0.3), trigger(0, 2.133)]),
        ];
        let combat = BTreeSet::from(["groundareaattackclip".to_string()]);

        let missing = survey(&objects, &combat, &names);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].local_time, 2.133);
    }

    #[test]
    fn the_sweep_wins_when_a_clip_carries_both_markers() {
        let names = names_map(&["AreaAttackStart", "WeaponSweepAttackStart"]);
        let objects = vec![
            clip("Both", 1),
            trigger_array(vec![trigger(0, 2.0), trigger(1, 1.0)]),
        ];
        let combat = BTreeSet::from(["both".to_string()]);

        let missing = survey(&objects, &combat, &names);

        assert_eq!(missing.len(), 1);
        assert_eq!(
            missing[0].local_time, 1.0,
            "WeaponSweepAttackStart outranks AreaAttackStart"
        );
    }

    #[test]
    fn non_combat_clips_are_out_of_scope() {
        let names = names_map(&["WeaponSweepAttackStart"]);
        let objects = vec![clip("SomeIdle", 1), trigger_array(vec![trigger(0, 1.2)])];
        let combat = BTreeSet::from(["wingswipeleft".to_string()]);

        assert!(survey(&objects, &combat, &names).is_empty());
    }

    #[test]
    fn a_clip_with_no_damage_window_is_not_given_an_invented_hit() {
        let names = names_map(&["CameraShake", "soundPlayAt"]);
        let objects = vec![
            clip("Roar", 1),
            trigger_array(vec![trigger(0, 0.3), trigger(1, 0.0)]),
        ];
        let combat = BTreeSet::from(["roar".to_string()]);

        assert!(survey(&objects, &combat, &names).is_empty());
    }

    #[test]
    fn an_animation_annotated_hit_is_told_apart_from_prehitframe() {
        // Every sweep clip also carries `preHitFrame`, so a substring test would read the
        // whole broken set as already-annotated and repair nothing.
        assert!(bytes_fire_hit_frame(b"\0weaponSwing\0HitFrame\0FootLeft\0"));
        assert!(!bytes_fire_hit_frame(
            b"\0preHitFrame\0WeaponSweepAttackStart\0"
        ));
        assert!(!bytes_fire_hit_frame(b"\0FootLeft\0SoundPlay.AttackA\0"));
    }

    #[test]
    fn the_donor_timing_basis_is_carried_over() {
        let names = names_map(&["WeaponSweepAttackStart"]);
        let mut triggers = trigger(0, -0.4);
        if let HkxValue::Object(members) = &mut triggers {
            for m in members.iter_mut() {
                if m.name == "relativeToEndOfClip" {
                    m.value = HkxValue::Bool(true);
                }
            }
        }
        let objects = vec![clip("Swipe", 1), trigger_array(vec![triggers])];
        let combat = BTreeSet::from(["swipe".to_string()]);

        let missing = survey(&objects, &combat, &names);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].relative_to_end_of_clip, HkxValue::Bool(true));
        let built = new_trigger(&missing[0], 7);
        let HkxValue::Object(members) = built else {
            panic!("expected an inline struct")
        };
        let rel = members
            .iter()
            .find(|m| m.name == "relativeToEndOfClip")
            .unwrap();
        assert_eq!(rel.value, HkxValue::Bool(true));
    }
}
