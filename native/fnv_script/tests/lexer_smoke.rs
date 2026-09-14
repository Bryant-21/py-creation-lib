use fnv_script_native::lexer::{TokenKind, tokenize};

#[test]
fn lex_simple_block() {
    let src = r#"
ScriptName MyScript
short var
Begin OnActivate
    Set var to 1
End
"#;

    let tokens: Vec<TokenKind> = tokenize(src)
        .unwrap()
        .into_iter()
        .map(|token| token.kind)
        .collect();

    assert!(
        tokens
            .iter()
            .any(|token| matches!(token, TokenKind::Keyword(k) if k == "ScriptName"))
    );
    assert!(
        tokens
            .iter()
            .any(|token| matches!(token, TokenKind::Keyword(k) if k == "Begin"))
    );
    assert!(
        tokens
            .iter()
            .any(|token| matches!(token, TokenKind::Keyword(k) if k == "End"))
    );
    assert!(
        tokens
            .iter()
            .any(|token| matches!(token, TokenKind::IntLit(1)))
    );
}

#[test]
fn control_flow_keywords_are_case_insensitive_without_prefix_matching_identifiers() {
    let tokens: Vec<TokenKind> =
        tokenize("bEgIn eLsEiF ElSe EnDiF ReTuRn SeT value To 1 eNd\nelseIfFlag ifCondition")
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect();

    assert_eq!(tokens[0], TokenKind::Keyword("Begin".into()));
    assert_eq!(tokens[1], TokenKind::Keyword("elseif".into()));
    assert_eq!(tokens[2], TokenKind::Keyword("else".into()));
    assert_eq!(tokens[3], TokenKind::Keyword("endif".into()));
    assert_eq!(tokens[4], TokenKind::Keyword("Return".into()));
    assert_eq!(tokens[5], TokenKind::Keyword("Set".into()));
    assert_eq!(tokens[6], TokenKind::Ident("value".into()));
    assert_eq!(tokens[7], TokenKind::Keyword("to".into()));
    assert_eq!(tokens[9], TokenKind::Keyword("End".into()));
    assert_eq!(tokens[11], TokenKind::Ident("elseIfFlag".into()));
    assert_eq!(tokens[12], TokenKind::Ident("ifCondition".into()));
}
