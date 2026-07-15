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
