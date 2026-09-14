use fnv_script_native::ast::Stmt;
use fnv_script_native::error::FnvScriptError;
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

#[test]
fn parse_initialized_variable_without_consuming_following_block() {
    let src = r#"
scn InitializedScript
Int nStart to 1
Begin OnTrigger Player
    If (nStart == 0)
        Set nStart to 1
    EndIf
End
"#;

    let script = parse_script(src).unwrap();
    assert_eq!(script.variables.len(), 1);
    assert_eq!(
        script.variables[0].initial,
        Some(fnv_script_native::ast::Expr::Int(1))
    );
    assert_eq!(script.blocks.len(), 1);
    assert_eq!(script.blocks[0].event, "OnTrigger");
}

#[test]
fn stray_top_level_terminator_returns_bounded_parse_error() {
    let error = parse_script("End\n").unwrap_err();

    assert!(matches!(error, FnvScriptError::Parse { .. }));
    assert!(error.to_string().contains("no forward progress"));
}
