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
