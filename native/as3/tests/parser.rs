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
fn a_dotted_package_name_is_kept_whole() {
    let unit = parse("package com.example.ui { public class A { } }").unwrap();
    assert_eq!(unit.packages[0].name, "com.example.ui");
}

#[test]
fn a_wildcard_import_is_flagged() {
    let unit = parse("package { import flash.display.*; public class A { } }").unwrap();
    assert!(unit.packages[0].imports[0].wildcard);
    assert_eq!(unit.packages[0].imports[0].name.joined(), "flash.display");
}

#[test]
fn modifiers_are_collected_in_any_order() {
    let class = one_class("package { dynamic public final class A { } }");
    assert_eq!(class.modifiers.visibility, Some(Visibility::Public));
    assert!(class.modifiers.is_dynamic);
    assert!(class.modifiers.is_final);
}

/// `static` is only a modifier when a declaration follows it; used as a name it
/// must stay an ordinary identifier.
#[test]
fn static_is_a_modifier_only_before_a_declaration() {
    let class = one_class("package { public class A { public static var n:int; } }");
    match &class.members[0] {
        Member::Var(v) => {
            assert!(v.modifiers.is_static);
            assert_eq!(v.name, "n");
        }
        other => panic!("expected a var member, got {other:?}"),
    }

    let class = one_class("package { public class A { public function f() { static = 1; } } }");
    match &class.members[0] {
        Member::Function(f) => {
            let body = f.body.as_ref().unwrap();
            assert!(matches!(
                body.statements[0],
                Stmt::Expr(Expr::Assign { .. })
            ));
        }
        other => panic!("expected a function member, got {other:?}"),
    }
}

#[test]
fn implements_and_extends_are_recorded_separately() {
    let class = one_class("package { public class A extends B implements C, D { } }");
    assert_eq!(class.extends.as_ref().unwrap().joined(), "B");
    let names: Vec<String> = class.implements.iter().map(DottedName::joined).collect();
    assert_eq!(names, ["C", "D"]);
}

/// An interface's `extends` list is a conformance list, not a superclass.
#[test]
fn an_interface_extends_list_becomes_implements() {
    let class = one_class("package { public interface A extends B, C { } }");
    assert!(class.is_interface);
    assert!(class.extends.is_none());
    let names: Vec<String> = class.implements.iter().map(DottedName::joined).collect();
    assert_eq!(names, ["B", "C"]);
}

#[test]
fn the_hudframework_widget_interface_parses() {
    let class = one_class(
        r#"
        package hudframework {
            public interface IHUDWidget {
                function processMessage(command:String, params:Array):void;
            }
        }
        "#,
    );
    assert!(class.is_interface);
    assert_eq!(class.name, "IHUDWidget");
    match &class.members[0] {
        Member::Function(f) => {
            assert_eq!(f.name, "processMessage");
            assert!(f.body.is_none(), "an interface method has no body");
            assert_eq!(f.sig.return_type, TypeRef::Void);
            let params: Vec<(&str, String)> = f
                .sig
                .params
                .iter()
                .map(|p| {
                    let ty = match &p.type_ref {
                        TypeRef::Named(n) => n.joined(),
                        TypeRef::Any => "*".into(),
                        TypeRef::Void => "void".into(),
                    };
                    (p.name.as_str(), ty)
                })
                .collect();
            assert_eq!(
                params,
                [
                    ("command", "String".to_string()),
                    ("params", "Array".to_string())
                ]
            );
        }
        other => panic!("expected a function member, got {other:?}"),
    }
}

#[test]
fn accessors_are_distinguished_from_a_method_named_get() {
    let class = one_class(
        "package { public class A { public function get width():int { return 0; } \
         public function get():int { return 0; } } }",
    );
    match (&class.members[0], &class.members[1]) {
        (Member::Function(a), Member::Function(b)) => {
            assert_eq!(a.accessor, Accessor::Getter);
            assert_eq!(a.name, "width");
            assert_eq!(b.accessor, Accessor::None);
            assert_eq!(b.name, "get");
        }
        other => panic!("expected two function members, got {other:?}"),
    }
}

#[test]
fn parameters_carry_types_defaults_and_rest() {
    let class = one_class(
        "package { public class A { public function f(a:int, b:String = \"x\", ...rest) { } } }",
    );
    let Member::Function(f) = &class.members[0] else {
        panic!("expected a function member");
    };
    assert_eq!(f.sig.params.len(), 3);
    assert!(f.sig.params[1].default.is_some());
    assert!(f.sig.params[2].is_rest);
    assert_eq!(f.sig.params[2].name, "rest");
}

/// Precedence, not just acceptance: `a + b * c` must nest the multiply under
/// the add, and comparison must bind looser than arithmetic.
#[test]
fn binary_operators_nest_by_precedence() {
    let class = one_class("package { public class A { public function f() { x = a + b * c; } } }");
    let Member::Function(f) = &class.members[0] else {
        panic!("expected a function member");
    };
    let Stmt::Expr(Expr::Assign { value, .. }) = &f.body.as_ref().unwrap().statements[0] else {
        panic!("expected an assignment");
    };
    let Expr::Binary { op, rhs, .. } = &**value else {
        panic!("expected a binary expression");
    };
    assert_eq!(*op, BinOp::Add);
    assert!(matches!(**rhs, Expr::Binary { op: BinOp::Mul, .. }));
}

#[test]
fn a_call_chain_parses_left_to_right() {
    let class = one_class("package { public class A { public function f() { a.b(1).c[2]; } } }");
    let Member::Function(f) = &class.members[0] else {
        panic!("expected a function member");
    };
    let Stmt::Expr(expr) = &f.body.as_ref().unwrap().statements[0] else {
        panic!("expected an expression statement");
    };
    // Outermost is the index, then the member `c`, then the call, then `a.b`.
    let Expr::Index { object, .. } = expr else {
        panic!("outermost should be an index");
    };
    let Expr::Member { object, name, .. } = &**object else {
        panic!("next should be a member access");
    };
    assert_eq!(name, "c");
    assert!(matches!(**object, Expr::Call { .. }));
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
    assert!(matches!(
        body.statements[0],
        Stmt::Unsupported { what: "for", .. }
    ));
    assert!(matches!(
        body.statements[1],
        Stmt::Unsupported { what: "switch", .. }
    ));
    // Parsing recovers: the statement after an unsupported one is still seen.
    assert!(matches!(body.statements[2], Stmt::Expr(Expr::Call { .. })));
}

#[test]
fn syntax_errors_report_what_was_expected() {
    let err = parse("package { public class A extends { } }").unwrap_err();
    assert!(err.message.contains("expected an identifier"), "{err}");

    let err = parse("class A { }").unwrap_err();
    assert!(err.message.contains("expected `package`"), "{err}");

    let err = parse("package { public class A { ").unwrap_err();
    assert!(err.message.contains("unterminated class body"), "{err}");
}
