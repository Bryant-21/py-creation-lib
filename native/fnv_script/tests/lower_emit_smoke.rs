use fnv_script_native::context::{FnvScriptContext, SymbolMetadata, TargetMetadata};
use fnv_script_native::emit::emit_psc;
use fnv_script_native::error::FnvScriptError;
use fnv_script_native::function_map::FunctionMap;
use fnv_script_native::lower::lower;
use fnv_script_native::parser::parse_script;
use std::collections::HashMap;

fn ctx(map_yaml: &str) -> FnvScriptContext {
    ctx_with_strict(map_yaml, true)
}

fn ctx_with_strict(map_yaml: &str, strict: bool) -> FnvScriptContext {
    let papyrus_extends = "ObjectReference".to_string();
    FnvScriptContext {
        function_map: FunctionMap::from_yaml(map_yaml).unwrap(),
        actor_value_map: HashMap::new(),
        mod_prefix: "B21_T".into(),
        strict,
        script_class_name: "B21_T_nv_Test".into(),
        target: TargetMetadata::for_extends(&papyrus_extends),
        papyrus_extends,
    }
}

#[test]
fn lowers_simple_set_and_call() {
    let src = r#"
ScriptName Test
short c
Begin OnActivate
    Set c to 1
End
"#;

    let ast = parse_script(src).unwrap();
    let ir = lower(&ast, &ctx("")).unwrap();
    let psc = emit_psc(&ir);
    assert!(psc.contains("ScriptName B21_T_nv_Test extends ObjectReference"));
    assert!(psc.contains("Int Property c Auto"));
    assert!(psc.contains("Event OnActivate(ObjectReference akActionRef)"));
    assert!(psc.contains("c = 1"));
}

#[test]
fn lowers_initialized_source_variable_as_initialized_property() {
    let ast = parse_script("Int nStart to 1\nBegin OnLoad\nEnd\n").unwrap();
    let psc = emit_psc(&lower(&ast, &ctx("")).unwrap());

    assert!(psc.contains("Int Property nStart = 1 Auto"));
}

#[test]
fn lowers_papyrus_function_call() {
    let src = r#"
Begin OnActivate
    Activate Player
End
"#;

    let ast = parse_script(src).unwrap();
    let map = r#"
Activate:
  papyrus: "{arg0}.Activate({self}, false)"
  arg_kinds: [actor]
  return_kind: void
Player:
  papyrus: "Game.GetPlayer()"
  arg_kinds: []
  return_kind: actor
"#;
    let ir = lower(&ast, &ctx(map)).unwrap();
    let psc = emit_psc(&ir);
    assert!(psc.contains("Game.GetPlayer().Activate(Self, false)"));
}

#[test]
fn strict_unmapped_function_errors() {
    let src = r#"
Begin OnActivate
    RewardKarma 10
End
"#;

    let ast = parse_script(src).unwrap();
    let err = lower(&ast, &ctx("")).unwrap_err();
    assert!(err.to_string().contains("RewardKarma"));
}

#[test]
fn strict_unmapped_zero_arg_symbol_errors() {
    let src = r#"
Begin OnActivate
    Activate Player
End
"#;

    let ast = parse_script(src).unwrap();
    let map = r#"
Activate:
  papyrus: "{arg0}.Activate({self}, false)"
  arg_kinds: [actor]
  return_kind: void
"#;
    let err = lower(&ast, &ctx(map)).unwrap_err();
    assert!(err.to_string().contains("Player"));
}

#[test]
fn gamemode_restarts_from_reference_lifecycle_events() {
    let src = r#"
Begin GameMode
    Set c to 1
End
"#;

    let ast = parse_script(src).unwrap();
    let ir = lower(&ast, &ctx("")).unwrap();
    let psc = emit_psc(&ir);
    assert!(psc.contains("Event OnInit()"));
    assert!(psc.contains("Event OnLoad()"));
    assert!(psc.contains("Event OnTimer(Int aiTimerID)"));
    assert!(psc.contains("StartTimer(1.0, 0)"));
}

#[test]
fn drop_with_warning_is_a_structured_error_even_when_not_strict() {
    let src = r#"
Begin OnActivate
    RewardKarma 10
End
"#;
    let map = r#"
RewardKarma:
  rewrite: drop_with_warning
  reason: "FO4 has no Karma system"
  strict_failure: false
  arg_kinds: [int]
"#;

    let ast = parse_script(src).unwrap();
    let err = lower(&ast, &ctx_with_strict(map, false)).unwrap_err();
    match err {
        FnvScriptError::Drop { kind, name, reason } => {
            assert_eq!(kind, "function");
            assert_eq!(name, "RewardKarma");
            assert!(reason.contains("Karma"));
        }
        other => panic!("expected drop error, got {other}"),
    }
}

#[test]
fn enforces_exact_arity_before_rendering() {
    let map = r#"
SetStage:
  papyrus: "{arg0}.SetStage({arg1})"
  arg_kinds: [quest, int]
  return_kind: void
"#;
    let mut exact_ctx = ctx(map);
    exact_ctx
        .target
        .insert_symbol("MyQuest", SymbolMetadata::new("MyQuest", "Quest"));

    let exact = lower(
        &parse_script("Begin OnActivate\nSetStage MyQuest 20\nEnd").unwrap(),
        &exact_ctx,
    )
    .unwrap();
    let psc = emit_psc(&exact);
    assert!(psc.contains("MyQuest.SetStage(20)"));
    assert!(!psc.contains("{arg"));

    let missing = lower(
        &parse_script("Begin OnActivate\nSetStage MyQuest\nEnd").unwrap(),
        &exact_ctx,
    )
    .unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("expected 2 argument(s), got 1")
    );

    let extra = lower(
        &parse_script("Begin OnActivate\nSetStage MyQuest 20 30\nEnd").unwrap(),
        &exact_ctx,
    )
    .unwrap_err();
    assert!(extra.to_string().contains("expected 2 argument(s), got 3"));
}

#[test]
fn explicit_static_record_kind_types_compact_custom_script_classes() {
    let map = r#"
SetStage:
  papyrus: "{arg0}.SetStage({arg1})"
  arg_kinds: [quest, int]
  return_kind: void
"#;
    let mut compact_ctx = ctx(map);
    compact_ctx.target.insert_symbol(
        "MyQuest",
        SymbolMetadata::new("MyQuest", "FNV_FO3_Merged_S_11FC64").with_static_record_kind("quest"),
    );
    let lowered = lower(
        &parse_script("Begin OnActivate\nSetStage MyQuest 20\nEnd").unwrap(),
        &compact_ctx,
    )
    .unwrap();
    assert!(emit_psc(&lowered).contains("MyQuest.SetStage(20)"));

    let mut untyped_ctx = ctx(map);
    untyped_ctx.target.insert_symbol(
        "MyQuest",
        SymbolMetadata::new("MyQuest", "FNV_FO3_Merged_S_11FC64"),
    );
    assert!(
        lower(
            &parse_script("Begin OnActivate\nSetStage MyQuest 20\nEnd").unwrap(),
            &untyped_ctx,
        )
        .unwrap_err()
        .to_string()
        .contains("kind cannot be proven")
    );
}

#[test]
fn rejects_statically_wrong_argument_kinds() {
    let map = r#"
SetStage:
  papyrus: "{arg0}.SetStage({arg1})"
  arg_kinds: [quest, int]
  return_kind: void
"#;
    let err = lower(
        &parse_script("Begin OnActivate\nSetStage 7 20\nEnd").unwrap(),
        &ctx(map),
    )
    .unwrap_err();
    assert!(err.to_string().contains("expected quest"));
    assert!(err.to_string().contains("known int"));
}

#[test]
fn non_strict_allows_only_statically_ambiguous_arguments() {
    let map = r#"
UseActor:
  papyrus: "{arg0}.EvaluatePackage(true)"
  arg_kinds: [actor]
  return_kind: void
"#;
    let source = parse_script("Begin OnActivate\nUseActor Carrier.SomeActor\nEnd").unwrap();
    let mut strict_ctx = ctx_with_strict(map, true);
    strict_ctx.target.insert_symbol(
        "Carrier",
        SymbolMetadata::new("Carrier", "Quest").with_member("SomeActor", "SomeActor"),
    );
    let err = lower(&source, &strict_ctx).unwrap_err();
    assert!(err.to_string().contains("cannot be proven"));

    let mut permissive_ctx = strict_ctx;
    permissive_ctx.strict = false;
    let psc = emit_psc(&lower(&source, &permissive_ctx).unwrap());
    assert!(psc.contains("Carrier.SomeActor.EvaluatePackage(true)"));

    let known_wrong = parse_script("Begin OnActivate\nUseActor 7\nEnd").unwrap();
    let err = lower(&known_wrong, &permissive_ctx).unwrap_err();
    assert!(err.to_string().contains("expected actor"));
}

#[test]
fn maps_audited_actor_values_and_fails_closed_for_unknown_values() {
    let map = r#"
GetActorValue:
  papyrus: "{self}.GetValue({arg0})"
  arg_kinds: [actor_value]
  return_kind: float
"#;
    let source = parse_script("Begin OnActivate\nGetActorValue Strength\nEnd").unwrap();
    let mut mapped_ctx = ctx(map);
    mapped_ctx
        .actor_value_map
        .insert("strength".into(), "Strength".into());
    let psc = emit_psc(&lower(&source, &mapped_ctx).unwrap());
    assert!(psc.contains("Self.GetValue(Strength)"));
    assert!(!psc.contains("{arg"));

    let unknown = parse_script("Begin OnActivate\nGetActorValue Karma\nEnd").unwrap();
    mapped_ctx.strict = false;
    let err = lower(&unknown, &mapped_ctx).unwrap_err();
    assert!(err.to_string().contains("unmapped actor value 'Karma'"));

    let wrong_kind = parse_script("Begin OnActivate\nGetActorValue 5\nEnd").unwrap();
    let err = lower(&wrong_kind, &mapped_ctx).unwrap_err();
    assert!(err.to_string().contains("expected actor_value"));
}
