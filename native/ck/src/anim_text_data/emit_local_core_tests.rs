use super::*;
use havok_native::hkx::types::HkxValue;
use havok_native::hkx::{HkxFile, HkxMember, HkxObject};

#[test]
fn scorched_unarmed_weapon_profile_keeps_local_attack_metadata() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let core = r"Actors\B21_FO76\Scorched\Behaviors\Melee.hkx";
    let event = "meleeH2HAttackStandingA";
    let member = |name: &str, value| HkxMember {
        name: name.into(),
        value,
    };
    let string = |value: &str| HkxValue::String {
        value: value.into(),
        is_null: false,
    };
    let object = |class: &str, members| HkxObject {
        name: None,
        offset: 0,
        signature: 0,
        class_name: class.into(),
        members,
    };
    let graph = HkxFile::from_tagxml(
        11,
        "hk_2014.1.0-r1",
        vec![
            object(
                "hkbBehaviorGraphStringData",
                vec![member("eventNames", HkxValue::Array(vec![string(event)]))],
            ),
            object(
                "hkbStateMachine",
                vec![
                    member("states", HkxValue::Array(vec![HkxValue::Pointer(Some(2))])),
                    member("wildcardTransitions", HkxValue::Pointer(Some(3))),
                ],
            ),
            object(
                "hkbStateMachineStateInfo",
                vec![
                    member("stateId", HkxValue::I32(0)),
                    member("generator", HkxValue::Pointer(Some(4))),
                ],
            ),
            object(
                "hkbStateMachineTransitionInfoArray",
                vec![member(
                    "transitions",
                    HkxValue::Array(vec![HkxValue::Object(vec![
                        member("eventId", HkxValue::I32(0)),
                        member("toStateId", HkxValue::I32(0)),
                    ])]),
                )],
            ),
            object(
                "hkbClipGenerator",
                vec![
                    member("name", string("H2HAttackStandingA")),
                    member(
                        "animationName",
                        string(r"Animations\H2HAttackStandingA.hkx"),
                    ),
                ],
            ),
        ],
    );
    let path = src.path().join(core.replace('\\', "/"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, graph.save()).unwrap();
    assert_eq!(resolve_anim_events(&path, &[event.into()]).len(), 1);

    let subgraph = SubgraphInput {
        core_behavior: core.into(),
        sapt_chain: vec![],
        race_dir: Some(r"Actors\B21_FO76\Scorched".into()),
    };
    // The same graph/SAPT ID serves the default unarmed and STKD-selected blocks.
    let weapon_ids = BTreeSet::from([subgraph.id()]);
    emit_derivable_buckets_with_progress(
        std::slice::from_ref(&subgraph),
        &weapon_ids,
        &[],
        &[event.into()],
        src.path(),
        out.path(),
        None,
        None,
        &mut |_| {},
    );
    let events = std::fs::read_to_string(
        out.path()
            .join("AnimTextData/AnimEventInfo")
            .join(format!("{}.txt", name_id(core))),
    )
    .expect("local weapon graph must emit attack-event metadata");
    assert!(events.contains(event));
    assert!(events.contains("H2HAttackStandingA"));
    let clips = std::fs::read(
        out.path()
            .join("AnimTextData/ClipGeneratorData")
            .join(format!("{}.txt", name_id(core))),
    )
    .unwrap();
    let clip_name = b"H2HAttackStandingA\0";
    assert!(clips.windows(clip_name.len()).any(|s| s == clip_name));

    let vanilla = SubgraphInput {
        core_behavior: r"Actors\Character\Behaviors\MeleeBehavior.hkx".into(),
        ..subgraph
    };
    emit_derivable_buckets_with_progress(
        std::slice::from_ref(&vanilla),
        &BTreeSet::from([vanilla.id()]),
        &[],
        &[event.into()],
        src.path(),
        out.path(),
        None,
        None,
        &mut |_| {},
    );
    for bucket in ["AnimEventInfo", "ClipGeneratorData"] {
        assert!(
            !out.path()
                .join("AnimTextData")
                .join(bucket)
                .join(format!("{}.txt", name_id(&vanilla.core_behavior)))
                .exists()
        );
    }
}
