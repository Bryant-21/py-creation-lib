//! Parser shape checks. These assert the tree, not just that parsing returned
//! `Ok` — a parser that silently drops a member would otherwise pass.

use as3_native::ast::*;
use as3_native::parser::parse;

fn one_class(src: &str) -> ClassDecl {
    let unit = parse(src).unwrap();
    assert_eq!(unit.packages.len(), 1);
    assert_eq!(unit.packages[0].classes.len(), 1);
    unit.packages[0].classes[0].clone()
}

fn first_statement(src: &str) -> Stmt {
    let class = one_class(src);
    let Member::Function(f) = &class.members[0] else {
        panic!("expected a function member");
    };
    f.body.as_ref().unwrap().statements[0].clone()
}

fn joined(names: &[DottedName]) -> Vec<String> {
    names.iter().map(DottedName::joined).collect()
}

#[test]
fn the_task_source_parses_to_the_expected_tree() {
    let unit = parse(
        r#"
        package {
            import flash.display.MovieClip;
            public class Foo extends MovieClip {
                public function Foo() { }
            }
        }
        "#,
    )
    .unwrap();

    let pkg = &unit.packages[0];
    assert_eq!(pkg.name, "");
    assert_eq!(pkg.imports.len(), 1);
    assert_eq!(pkg.imports[0].name.joined(), "flash.display.MovieClip");
    assert!(!pkg.imports[0].wildcard);

    let class = &pkg.classes[0];
    assert_eq!(class.name, "Foo");
    assert_eq!(class.modifiers.visibility, Some(Visibility::Public));
    assert!(!class.modifiers.is_dynamic);
    assert_eq!(
        class.extends.as_ref().map(DottedName::joined).as_deref(),
        Some("MovieClip")
    );
    assert_eq!(class.members.len(), 1);
    match &class.members[0] {
        Member::Function(f) => {
            assert_eq!(f.name, "Foo");
            assert!(f.sig.params.is_empty());
            assert_eq!(f.sig.return_type, TypeRef::Any);
            assert!(f.body.as_ref().unwrap().statements.is_empty());
        }
        other => panic!("expected a function member, got {other:?}"),
    }
}

#[test]
fn package_and_class_headers_are_recorded() {
    let unit = parse("package com.example.ui { import flash.display.*; public class A { } }").unwrap();
    assert_eq!(unit.packages[0].name, "com.example.ui");
    assert!(unit.packages[0].imports[0].wildcard);
    assert_eq!(unit.packages[0].imports[0].name.joined(), "flash.display");

    let class = one_class("package { dynamic public final class A { } }");
    assert_eq!(class.modifiers.visibility, Some(Visibility::Public));
    assert!(class.modifiers.is_dynamic && class.modifiers.is_final);

    let class = one_class("package { public class A extends B implements C, D { } }");
    assert_eq!(class.extends.as_ref().unwrap().joined(), "B");
    assert_eq!(joined(&class.implements), ["C", "D"]);

    // An interface's `extends` list is a conformance list, not a superclass.
    let class = one_class("package { public interface A extends B, C { } }");
    assert!(class.is_interface);
    assert!(class.extends.is_none());
    assert_eq!(joined(&class.implements), ["B", "C"]);
}

#[test]
fn members_carry_modifiers_accessors_and_signatures() {
    let class = one_class(
        "package { public class A { public static var n:int; \
         public function get width():int { return 0; } \
         public function get():int { return 0; } \
         public function f(a:int, b:String = \"x\", ...rest) { } } }",
    );
    let Member::Var(v) = &class.members[0] else {
        panic!("expected a var member");
    };
    assert!(v.modifiers.is_static);
    assert_eq!(v.name, "n");
    let [Member::Function(getter), Member::Function(get), Member::Function(f)] =
        &class.members[1..]
    else {
        panic!("expected three function members");
    };
    assert_eq!((getter.accessor, getter.name.as_str()), (Accessor::Getter, "width"));
    assert_eq!((get.accessor, get.name.as_str()), (Accessor::None, "get"));
    assert_eq!(f.sig.params.len(), 3);
    assert!(f.sig.params[1].default.is_some());
    assert!(f.sig.params[2].is_rest);
    assert_eq!(f.sig.params[2].name, "rest");

    let class = one_class(
        "package hudframework { public interface IHUDWidget { \
         function processMessage(command:String, params:Array):void; } }",
    );
    let Member::Function(m) = &class.members[0] else {
        panic!("expected a function member");
    };
    assert_eq!(m.name, "processMessage");
    assert!(m.body.is_none(), "an interface method has no body");
    assert_eq!(m.sig.return_type, TypeRef::Void);
    let params: Vec<(&str, String)> = m
        .sig
        .params
        .iter()
        .map(|p| match &p.type_ref {
            TypeRef::Named(n) => (p.name.as_str(), n.joined()),
            other => panic!("unexpected type {other:?}"),
        })
        .collect();
    assert_eq!(
        params,
        [("command", "String".to_string()), ("params", "Array".to_string())]
    );
}

#[test]
fn expressions_nest_by_precedence_and_chain_left_to_right() {
    let Stmt::Expr(Expr::Assign { value, .. }) =
        first_statement("package { public class A { public function f() { x = a + b * c; } } }")
    else {
        panic!("expected an assignment");
    };
    let Expr::Binary { op, rhs, .. } = &*value else {
        panic!("expected a binary expression");
    };
    assert_eq!(*op, BinOp::Add);
    assert!(matches!(**rhs, Expr::Binary { op: BinOp::Mul, .. }));

    // Outermost is the index, then the member `c`, then the call, then `a.b`.
    let Stmt::Expr(Expr::Index { object, .. }) =
        first_statement("package { public class A { public function f() { a.b(1).c[2]; } } }")
    else {
        panic!("outermost should be an index");
    };
    let Expr::Member { object, name, .. } = &*object else {
        panic!("next should be a member access");
    };
    assert_eq!(name, "c");
    assert!(matches!(**object, Expr::Call { .. }));

    // `static` used as a name stays an ordinary identifier.
    assert!(matches!(
        first_statement("package { public class A { public function f() { static = 1; } } }"),
        Stmt::Expr(Expr::Assign { .. })
    ));
}

#[test]
fn statements_this_phase_will_not_lower_are_marked_not_dropped() {
    let class = one_class(
        "package { public class A { public function f() { \
         for (var i:int = 0; i < 3; i++) { g(); } switch (x) { case 1: break; } h(); } } }",
    );
    let Member::Function(f) = &class.members[0] else {
        panic!("expected a function member");
    };
    let body = f.body.as_ref().unwrap();
    assert!(matches!(body.statements[0], Stmt::For { .. }));
    assert!(matches!(
        body.statements[1],
        Stmt::Unsupported { what: "switch", .. }
    ));
    // Parsing recovers: the statement after an unsupported one is still seen.
    assert!(matches!(body.statements[2], Stmt::Expr(Expr::Call { .. })));
}

#[test]
fn syntax_errors_report_what_was_expected() {
    for (src, needle) in [
        ("package { public class A extends { } }", "expected an identifier"),
        ("class A { }", "expected `package`"),
        ("package { public class A { ", "unterminated class body"),
    ] {
        let err = parse(src).unwrap_err();
        assert!(err.message.contains(needle), "{src}: {err}");
    }
}
