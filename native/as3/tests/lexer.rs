//! Tokenizer behaviour, with an eye on the cases that quietly corrupt a parse:
//! contextual keywords, the maximal-munch operators, and escapes.

use as3_native::lexer::{Keyword, Punct, TokenKind, tokenize};

fn kinds(src: &str) -> Vec<TokenKind> {
    let mut k: Vec<TokenKind> = tokenize(src).unwrap().into_iter().map(|t| t.kind).collect();
    assert_eq!(k.pop(), Some(TokenKind::Eof), "stream must end with Eof");
    k
}

fn puncts(src: &str) -> Vec<Punct> {
    kinds(src)
        .into_iter()
        .map(|k| match k {
            TokenKind::Punct(p) => p,
            other => panic!("expected punctuation, got {other:?}"),
        })
        .collect()
}

#[test]
fn the_declaration_header_lexes_to_the_expected_stream() {
    assert_eq!(
        kinds("package { public class Foo extends MovieClip {} }"),
        [
            TokenKind::Keyword(Keyword::Package),
            TokenKind::Punct(Punct::LBrace),
            TokenKind::Keyword(Keyword::Public),
            TokenKind::Keyword(Keyword::Class),
            TokenKind::Ident("Foo".into()),
            TokenKind::Keyword(Keyword::Extends),
            TokenKind::Ident("MovieClip".into()),
            TokenKind::Punct(Punct::LBrace),
            TokenKind::Punct(Punct::RBrace),
            TokenKind::Punct(Punct::RBrace),
        ]
    );
}

/// `static`, `dynamic`, `get` and friends are only keywords in the position
/// that expects them, so the lexer must hand them back as identifiers.
#[test]
fn contextual_keywords_lex_as_identifiers() {
    for word in [
        "static",
        "dynamic",
        "final",
        "override",
        "get",
        "set",
        "each",
        "namespace",
    ] {
        assert_eq!(
            kinds(word),
            [TokenKind::Ident(word.into())],
            "{word} should not be reserved"
        );
    }
}

#[test]
fn reserved_words_lex_as_keywords() {
    for (word, kw) in [
        ("public", Keyword::Public),
        ("internal", Keyword::Internal),
        ("native", Keyword::Native),
        ("is", Keyword::Is),
        ("as", Keyword::As),
    ] {
        assert_eq!(kinds(word), [TokenKind::Keyword(kw)], "{word}");
    }
}

#[test]
fn operators_use_maximal_munch() {
    assert_eq!(
        puncts("=== == = !== != ! >>> >> > <= << ++ += ...  ::"),
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
    );
}

#[test]
fn numbers_split_into_integers_and_doubles() {
    assert_eq!(kinds("0"), [TokenKind::Int(0)]);
    assert_eq!(kinds("42"), [TokenKind::Int(42)]);
    assert_eq!(kinds("0xFF00AA"), [TokenKind::Int(0xFF_00AA)]);
    assert_eq!(kinds("1.5"), [TokenKind::Number(1.5)]);
    assert_eq!(kinds("1e3"), [TokenKind::Number(1000.0)]);
    assert_eq!(kinds("1.5e-2"), [TokenKind::Number(0.015)]);
    // A decimal literal too wide for i64 is a double in AS3, not an error.
    assert_eq!(
        kinds("99999999999999999999"),
        [TokenKind::Number(99999999999999999999.0)]
    );
}

/// `1e` is not an exponent, so the `e` has to be given back as an identifier
/// rather than swallowed into a malformed number.
#[test]
fn a_bare_e_is_not_an_exponent() {
    assert_eq!(
        kinds("1eight"),
        [TokenKind::Int(1), TokenKind::Ident("eight".into())]
    );
}

#[test]
fn a_member_access_on_an_integer_keeps_the_dot() {
    assert_eq!(
        kinds("1.toString()"),
        [
            TokenKind::Int(1),
            TokenKind::Punct(Punct::Dot),
            TokenKind::Ident("toString".into()),
            TokenKind::Punct(Punct::LParen),
            TokenKind::Punct(Punct::RParen),
        ]
    );
}

#[test]
fn string_escapes_are_decoded() {
    assert_eq!(
        kinds(r#" "a\nb\t\"c\\" "#),
        [TokenKind::Str("a\nb\t\"c\\".into())]
    );
    assert_eq!(kinds(r#" 'é' "#), [TokenKind::Str("é".into())]);
    assert_eq!(kinds(r#" "\x41" "#), [TokenKind::Str("A".into())]);
}

#[test]
fn comments_and_whitespace_are_skipped() {
    assert_eq!(
        kinds("a // trailing\n /* block\n spanning */ b"),
        [TokenKind::Ident("a".into()), TokenKind::Ident("b".into())]
    );
}

/// HUDFramework's own `IHUDWidget.as` starts with a UTF-8 BOM, which would
/// otherwise lex as the first character of an identifier.
#[test]
fn a_leading_byte_order_mark_is_ignored() {
    assert_eq!(
        kinds("\u{feff}package"),
        [TokenKind::Keyword(Keyword::Package)]
    );
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
