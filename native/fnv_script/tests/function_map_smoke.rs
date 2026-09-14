use fnv_script_native::function_map::{EntryShape, FunctionMap};

#[test]
fn loads_three_entry_shapes() {
    let yaml = r#"
GetIsID:
  papyrus: "({arg0} == {self})"
  arg_kinds: [formkey]
  return_kind: bool

StartConversation:
  expansion: |
    Self.SetTopic({arg1})
    {arg0}.StartDialogue(Self)
  arg_kinds: [actor, topic]
  return_kind: void

RewardKarma:
  rewrite: drop_with_warning
  reason: "FO4 has no Karma system"
  strict_failure: true
"#;

    let map = FunctionMap::from_yaml(yaml).unwrap();
    assert!(matches!(
        map.get("GetIsID").unwrap().shape,
        EntryShape::Papyrus { .. }
    ));
    assert!(matches!(
        map.get("StartConversation").unwrap().shape,
        EntryShape::Expansion { .. }
    ));
    assert!(matches!(
        map.get("RewardKarma").unwrap().shape,
        EntryShape::Drop {
            strict_failure: true,
            ..
        }
    ));
    assert_eq!(map.len(), 3);
}

#[test]
fn rejects_templates_that_drop_or_invent_arguments() {
    let dropped = r#"
AddScriptPackage:
  papyrus: "{self}.EvaluatePackage(true)"
  arg_kinds: [package]
  return_kind: void
"#;
    let err = FunctionMap::from_yaml(dropped).unwrap_err();
    assert!(err.to_string().contains("does not consume"));

    let invented = r#"
GetPlayer:
  papyrus: "Game.GetPlayer({arg0})"
  arg_kinds: []
  return_kind: actor
"#;
    let err = FunctionMap::from_yaml(invented).unwrap_err();
    assert!(err.to_string().contains("declares 0 argument"));
}

#[test]
fn rejects_malformed_placeholders_and_unknown_argument_kinds() {
    let malformed = r#"
Activate:
  papyrus: "{argx}.Activate({self})"
  arg_kinds: [actor]
  return_kind: void
"#;
    let err = FunctionMap::from_yaml(malformed).unwrap_err();
    assert!(err.to_string().contains("malformed argument placeholder"));

    let unknown_kind = r#"
Activate:
  papyrus: "{arg0}.Activate({self})"
  arg_kinds: [anything]
  return_kind: void
"#;
    let err = FunctionMap::from_yaml(unknown_kind).unwrap_err();
    assert!(err.to_string().contains("unsupported arg kind"));
}
