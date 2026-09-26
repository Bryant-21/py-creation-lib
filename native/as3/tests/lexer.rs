//! Tokenizer behaviour, with an eye on the cases that quietly corrupt a parse:
//! contextual keywords, the maximal-munch operators, and escapes.

use as3_native::lexer::{Keyword, Punct, TokenKind, tokenize};

fn kinds(src: &str) -> Vec<TokenKind> {
    let mut k: Vec<TokenKind> = tokenize(src).unwrap().into_iter().map(|t| t.kind).collect();
    assert_eq!(k.pop(), Some(TokenKind::Eof), "stream must end with Eof");
    k
}

fn ident(s: &str) -> TokenKind {
    TokenKind::Ident(s.into())
}

#[test]
fn lexes_representative_inputs() {
    use TokenKind::{Int, Keyword as K, Number, Punct as P, Str};
    let cases: Vec<(&str, Vec<TokenKind>)> = vec![
        (
            "package { public class Foo extends MovieClip {} }",
            vec![
                K(Keyword::Package),
                P(Punct::LBrace),
                K(Keyword::Public),
                K(Keyword::Class),
                ident("Foo"),
                K(Keyword::Extends),
                ident("MovieClip"),
                P(Punct::LBrace),
                P(Punct::RBrace),
                P(Punct::RBrace),
            ],
        ),
        // Contextual keywords are only keywords where expected, so they lex as identifiers.
        (
            "static dynamic final override get set each namespace",
            ["static", "dynamic", "final", "override", "get", "set", "each", "namespace"]
                .map(ident)
                .to_vec(),
        ),
        (
            "internal native is as",
            vec![
                K(Keyword::Internal),
                K(Keyword::Native),
                K(Keyword::Is),
                K(Keyword::As),
            ],
        ),
        (
            "=== == = !== != ! >>> >> > <= << ++ += ...  ::",
            [
                Punct::StrictEq,
                Punct::Eq,
                Punct::Assign,
                Punct::StrictNe,
                Punct::Ne,
                Punct::Not,
                Punct::UShr,
                Punct::Shr,
                Punct::Gt,
                Punct::Le,
                Punct::Shl,
                Punct::PlusPlus,
                Punct::PlusAssign,
                Punct::DotDotDot,
                Punct::ColonColon,
            ]
            .map(P)
            .to_vec(),
        ),
        ("0xFF00AA", vec![Int(0xFF_00AA)]),
        ("1.5e-2", vec![Number(0.015)]),
        // A decimal literal too wide for i64 is a double in AS3, not an error.
        ("99999999999999999999", vec![Number(99999999999999999999.0)]),
        // `1e` is not an exponent: the `e` is given back as an identifier.
        ("1eight", vec![Int(1), ident("eight")]),
        (
            "1.toString()",
            vec![
                Int(1),
                P(Punct::Dot),
                ident("toString"),
                P(Punct::LParen),
                P(Punct::RParen),
            ],
        ),
        (r#" "a\nb\t\"c\\" "#, vec![Str("a\nb\t\"c\\".into())]),
        (r#" 'é' "#, vec![Str("é".into())]),
        (r#" "\x41" "#, vec![Str("A".into())]),
        (
            "a // trailing\n /* block\n spanning */ b",
            vec![ident("a"), ident("b")],
        ),
        // HUDFramework's IHUDWidget.as starts with a UTF-8 BOM.
        ("\u{feff}package", vec![K(Keyword::Package)]),
    ];
    for (src, want) in cases {
        assert_eq!(kinds(src), want, "{src:?}");
    }
}

#[test]
fn positions_are_one_based_and_track_newlines() {
    let tokens = tokenize("package\n  class").unwrap();
    assert_eq!(
        (tokens[0].span.start.line, tokens[0].span.start.col),
        (1, 1)
    );
    assert_eq!(
        (tokens[1].span.start.line, tokens[1].span.start.col),
        (2, 3)
    );
}

#[test]
fn malformed_input_is_reported_with_a_position() {
    for (src, needle) in [
        ("\"unterminated", "unterminated string"),
        ("/* unterminated", "unterminated block comment"),
        ("0x", "no digits"),
        ("#", "unexpected character"),
    ] {
        let err = tokenize(src).unwrap_err();
        assert!(err.message.contains(needle), "{src}: {err}");
        assert_eq!(err.span.start.line, 1, "{src}");
    }
}
