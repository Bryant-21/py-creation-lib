use fnv_script_native::ast::Stmt;
use fnv_script_native::parser::parse_script;

#[test]
fn parse_simple_object_script() {
    let src = r#"
ScriptName TestScript
short counter
Begin OnActivate
    Set counter to 1
    if counter == 1
        Activate Player
    endif
End
"#;

    let script = parse_script(src).unwrap();
    assert_eq!(script.name.as_deref(), Some("TestScript"));
    assert_eq!(script.variables.len(), 1);
    assert_eq!(script.blocks.len(), 1);
    let block = &script.blocks[0];
    assert_eq!(block.event, "OnActivate");
    assert_eq!(block.statements.len(), 2);
}

#[test]
fn parse_quest_script_with_set_to_function_call() {
    let src = r#"
ScriptName QuestScr
ref rPlayer
Begin GameMode
    Set rPlayer to GetPlayer
End
"#;

    let script = parse_script(src).unwrap();
    let block = &script.blocks[0];
    assert!(matches!(&block.statements[0], Stmt::Set { .. }));
}
