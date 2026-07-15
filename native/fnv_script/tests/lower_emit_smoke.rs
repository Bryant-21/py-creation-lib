use fnv_script_native::context::FnvScriptContext;
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
    FnvScriptContext {
        function_map: FunctionMap::from_yaml(map_yaml).unwrap(),
        actor_value_map: HashMap::new(),
        mod_prefix: "B21_T".into(),
        strict,
        script_class_name: "B21_T_nv_Test".into(),
        papyrus_extends: "ObjectReference".into(),
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
    assert!(psc.contains("Event OnActivate(ObjectReference akActionRef)"));
    assert!(psc.contains("c = 1"));
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
fn gamemode_maps_to_on_init() {
    let src = r#"
Begin GameMode
    Set c to 1
End
"#;

    let ast = parse_script(src).unwrap();
    let ir = lower(&ast, &ctx("")).unwrap();
    let psc = emit_psc(&ir);
    assert!(psc.contains("Event OnInit()"));
    assert!(!psc.contains("Event OnUpdate()"));
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
