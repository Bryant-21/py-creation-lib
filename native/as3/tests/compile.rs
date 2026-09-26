//! AS3 source text through to an ABC block.
//!
//! The byte-exact lock here is authoritative for a single class: its scope
//! depths and script-initialiser shape match what `WeaponCND.swf` (a
//! HUDFramework widget the engine loads) declares for its `MovieClip` subclass.
//! `swf_native::class_abc` reaches the same bytes by compiling generated AS3,
//! as `native/swf/tests/as3_equivalence.rs` asserts.

use as3_native::{Stage, compile_source, compile_to_do_abc};

const DYNAMIC_MAIN: &str = r#"
package {
    import flash.display.MovieClip;
    public dynamic class Main extends MovieClip {
        public function Main() { }
    }
}
"#;

/// The task's minimal-but-real source, verbatim.
const SEALED_FOO: &str = r#"
package {
    import flash.display.MovieClip;
    public class Foo extends MovieClip {
        public function Foo() { }
    }
}
"#;

/// Full byte lock. The scope depths and the script-initialiser shape here are
/// not invented: `WeaponCND.swf`'s own `Main` — a `MovieClip` subclass the
/// engine loads — declares `iinit` at scope 10..11, `cinit` at 9, and a
/// 39-byte script initialiser running at scope 1..9. All four match below.
#[rustfmt::skip]
const EXPECTED_DYNAMIC_MAIN: &[u8] = &[
    0x10, 0x00, 0x2E, 0x00, // minor 16, major 46
    0x00, 0x00, 0x00,       // int / uint / double pools: empty
    0x0C,                   // string_count 12 => 11 entries
    0x0D, b'f', b'l', b'a', b's', b'h', b'.', b'd', b'i', b's', b'p', b'l', b'a', b'y',
    0x09, b'M', b'o', b'v', b'i', b'e', b'C', b'l', b'i', b'p',
    0x00,                   // "" — the unnamed package's namespace name
    0x06, b'O', b'b', b'j', b'e', b'c', b't',
    0x0C, b'f', b'l', b'a', b's', b'h', b'.', b'e', b'v', b'e', b'n', b't', b's',
    0x0F, b'E', b'v', b'e', b'n', b't', b'D', b'i', b's', b'p', b'a', b't', b'c', b'h',
          b'e', b'r',
    0x0D, b'D', b'i', b's', b'p', b'l', b'a', b'y', b'O', b'b', b'j', b'e', b'c', b't',
    0x11, b'I', b'n', b't', b'e', b'r', b'a', b'c', b't', b'i', b'v', b'e', b'O', b'b',
          b'j', b'e', b'c', b't',
    0x16, b'D', b'i', b's', b'p', b'l', b'a', b'y', b'O', b'b', b'j', b'e', b'c', b't',
          b'C', b'o', b'n', b't', b'a', b'i', b'n', b'e', b'r',
    0x06, b'S', b'p', b'r', b'i', b't', b'e',
    0x04, b'M', b'a', b'i', b'n',
    0x04,                   // namespace_count 4 => 3 entries
    0x16, 0x01,             // PackageNamespace "flash.display"
    0x16, 0x03,             // PackageNamespace ""
    0x16, 0x05,             // PackageNamespace "flash.events"
    0x00,                   // ns_set_count
    0x09,                   // multiname_count 9 => 8 entries
    0x07, 0x01, 0x02,       // QName flash.display::MovieClip
    0x07, 0x02, 0x04,       // QName ::Object
    0x07, 0x03, 0x06,       // QName flash.events::EventDispatcher
    0x07, 0x01, 0x07,       // QName flash.display::DisplayObject
    0x07, 0x01, 0x08,       // QName flash.display::InteractiveObject
    0x07, 0x01, 0x09,       // QName flash.display::DisplayObjectContainer
    0x07, 0x01, 0x0A,       // QName flash.display::Sprite
    0x07, 0x02, 0x0B,       // QName ::Main
    0x03,                   // method_count: iinit, cinit, script init
    0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
    0x00,                   // metadata_count
    0x01,                   // class_count
    0x08, 0x01, 0x00, 0x00, 0x00, 0x00, // instance: Main : MovieClip, dynamic, iinit 0
    0x01, 0x00,             // class: cinit 1, no traits
    0x01,                   // script_count
    0x02,                   // script init = method 2
    0x01,                   // one trait
    0x08, 0x04, 0x01, 0x00, // Class trait: Main, slot 1, class 0
    0x03,                   // method_body_count
    // iinit: max_stack 1, locals 1, scope 10..11 — WeaponCND's Main to the byte
    0x00, 0x01, 0x01, 0x0A, 0x0B, 0x06, 0xD0, 0x30, 0xD0, 0x49, 0x00, 0x47, 0x00, 0x00,
    // cinit: returnvoid only, at the captured depth 9
    0x01, 0x00, 0x01, 0x09, 0x09, 0x01, 0x47, 0x00, 0x00,
    // script init: global scope, then MovieClip's seven ancestors, then the class
    0x02, 0x02, 0x01, 0x01, 0x09, 0x27,
    0xD0, 0x30, 0x65, 0x00,
    0x60, 0x02, 0x30, 0x60, 0x03, 0x30, 0x60, 0x04, 0x30, 0x60, 0x05, 0x30,
    0x60, 0x06, 0x30, 0x60, 0x07, 0x30, 0x60, 0x01, 0x30,
    0x60, 0x01, 0x58, 0x00,
    0x1D, 0x1D, 0x1D, 0x1D, 0x1D, 0x1D, 0x1D,
    0x68, 0x08, 0x47, 0x00, 0x00,
];

#[test]
fn a_dynamic_class_compiles_to_the_locked_bytes() {
    assert_eq!(compile_source(DYNAMIC_MAIN).unwrap(), EXPECTED_DYNAMIC_MAIN);
    let explicit_super =
        compile_source(&DYNAMIC_MAIN.replace("Main() { }", "Main() { super(); }")).unwrap();
    assert_eq!(explicit_super, EXPECTED_DYNAMIC_MAIN);
}

/// AS3 classes are sealed unless declared `dynamic`, so the *only* thing that
/// may change between the two is the `instance_info` flags byte. Sealed is not a
/// guess: `WeaponCND.swf`'s document class carries flags 0x09 (`CLASS_SEALED` set).
#[test]
fn sealing_changes_exactly_the_instance_flags_byte() {
    let dynamic = compile_source(DYNAMIC_MAIN).unwrap();
    let sealed =
        compile_source(&DYNAMIC_MAIN.replace("public dynamic class", "public class")).unwrap();

    assert_eq!(dynamic.len(), sealed.len());
    let differing: Vec<usize> = (0..dynamic.len())
        .filter(|&i| dynamic[i] != sealed[i])
        .collect();
    assert_eq!(differing.len(), 1, "expected exactly one differing byte");

    let at = differing[0];
    assert_eq!(dynamic[at], 0x00, "dynamic class carries no flags");
    assert_eq!(sealed[at], 0x01, "sealed class sets CLASS_SEALED");
    // `Main` is multiname 8, `MovieClip` multiname 1: the flags follow name then super_name.
    assert_eq!(&sealed[at - 2..at], &[0x08, 0x01], "name then super_name");

    let foo = compile_source(SEALED_FOO).unwrap();
    // "Foo" is one byte shorter than "Main".
    assert_eq!(foo.len(), EXPECTED_DYNAMIC_MAIN.len() - 1);
    assert!(foo.windows(3).any(|w| w == b"Foo"));
}

#[test]
fn the_do_abc_tag_body_carries_the_reference_header() {
    let body = compile_to_do_abc(SEALED_FOO).unwrap();
    // flags = 1 (lazy init), empty NUL-terminated name, then ABC 46.16.
    assert_eq!(&body[..5], &[0x01, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(&body[5..9], &[0x10, 0x00, 0x2E, 0x00]);
    assert_eq!(body.len(), compile_source(SEALED_FOO).unwrap().len() + 5);
}

fn contains(abc: &[u8], needle: &[u8]) -> bool {
    abc.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn packages_and_base_classes_resolve_into_the_pool() {
    // A named package lands in the class namespace (what a `SymbolClass` entry needs).
    let abc = compile_source(
        "package com.example.widgets { import flash.display.MovieClip; \
         public class Widget extends MovieClip { public function Widget() { } } }",
    )
    .unwrap();
    assert!(contains(&abc, b"com.example.widgets") && contains(&abc, b"Widget"));

    let abc = compile_source(
        r#"
        package {
            import flash.display.MovieClip;
            public dynamic class Alpha extends MovieClip { public function Alpha() { } }
            public dynamic class Beta extends MovieClip { public function Beta() { } }
        }
        "#,
    )
    .unwrap();
    assert!(contains(&abc, b"Alpha") && contains(&abc, b"Beta"));
    // The pool interns the shared base class rather than storing it twice.
    assert_eq!(
        abc.windows(13).filter(|w| *w == b"flash.display").count(),
        1
    );
    assert_eq!(abc.windows(9).filter(|w| *w == b"MovieClip").count(), 1);

    let abc = compile_source(
        "package { public class Foo extends flash.display.MovieClip { public function Foo() { } } }",
    )
    .unwrap();
    assert!(contains(&abc, b"flash.display"));

    let abc = compile_source("package { public class Foo { public function Foo() { } } }").unwrap();
    assert!(contains(&abc, b"Object"));
}

#[test]
fn supported_member_forms_are_emitted() {
    for member in [
        "public var count:int;",
        "public static function f():void { }",
        "public function get width():int { return 0; }",
        "override public function f():void { }",
        "private function f():void { }",
    ] {
        let src = format!(
            "package {{ import flash.display.MovieClip; public class Foo extends MovieClip {{ public function Foo() {{ }} {member} }} }}"
        );
        compile_source(&src).unwrap();
    }
}

#[test]
fn unsupported_or_unresolvable_sources_are_refused() {
    let cases: Vec<(String, Stage, &[&str])> = vec![
        // No import and no qualification: the name has nowhere to come from.
        (
            "package { public class Foo extends MovieClip { public function Foo() { } } }".into(),
            Stage::Unsupported,
            &["cannot resolve type `MovieClip`"],
        ),
        (
            "package { import flash.display.*; \
             public class Foo extends MovieClip { public function Foo() { } } }"
                .into(),
            Stage::Unsupported,
            &["wildcard import", "flash.display.*"],
        ),
        // Unknown ancestry cannot get a correct scope chain, so it is not guessed.
        (
            "package { import a.b.Unknown; \
             public class Foo extends Unknown { public function Foo() { } } }"
                .into(),
            Stage::Unsupported,
            &["no known ancestry"],
        ),
        (
            DYNAMIC_MAIN.replace("Main() { }", "Main() { with (this) { } }"),
            Stage::Unsupported,
            &["`with` statements"],
        ),
        (
            DYNAMIC_MAIN.replace("Main() { }", "Main():void { }"),
            Stage::Parse,
            &["return type"],
        ),
    ];
    for (src, stage, needles) in cases {
        let err = compile_source(&src).unwrap_err();
        assert_eq!(err.stage, stage, "{src}");
        for needle in needles {
            assert!(err.message.contains(needle), "{err}");
        }
    }
}

#[test]
fn diagnostics_carry_the_line_and_column_of_the_offending_construct() {
    let err = compile_source(
        "package {\n    import flash.display.MovieClip;\n    public class Foo extends Missing {\n        public function Foo() { }\n    }\n}",
    )
    .unwrap_err();
    assert_eq!(err.span.start.line, 3);
    assert_eq!(
        format!("{err}"),
        "3:30: unsupported: cannot resolve type `Missing`: \
         add `import <package>.Missing;` or write the name in full"
    );
}
