//! Bytecode-level checks on compiled method bodies.
//!
//! The emitted ABC is read back through `swf_native::abc` and its instruction
//! bytes asserted. No AVM2 runtime is available, so the reference is the AVM2
//! Overview's opcode table and branch rule (an `s24` operand is relative to the
//! byte after it); every branch target below is decoded and checked to land on
//! its intended instruction.

use as3_native::compile_source;
use swf_native::abc::{AbcBody, DO_ABC_DEFINE, parse_abc_detail};
use swf_native::class_abc::do_abc_define_body;

/// Compile a class body and return the named method's body plus its
/// `method_info` signature and the DoABC tag body.
fn method_in(members: &str, name: &str) -> (AbcBody, Vec<String>, String, Vec<u8>) {
    let src = format!("package {{ public class Foo {{ {members} }} }}");
    let abc = compile_source(&src).unwrap_or_else(|e| panic!("compiling {src}: {e}"));
    let tag = do_abc_define_body(&abc);
    let detail = parse_abc_detail(DO_ABC_DEFINE, &tag).unwrap();
    let class = detail.class("Foo").expect("class Foo");
    let t = class
        .instance_traits
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no method trait {name}"));
    let sig = &detail.methods[t.index as usize];
    (
        detail.body(t.index).expect("method body").clone(),
        sig.param_types.clone(),
        sig.return_type.clone(),
        tag,
    )
}

fn method(members: &str) -> AbcBody {
    method_in(members, "f").0
}

/// Decode the `s24` operand of the branch at `at`, returning the byte offset it
/// targets.
fn branch_target(code: &[u8], at: usize) -> usize {
    let raw = i32::from_le_bytes([code[at + 1], code[at + 2], code[at + 3], 0]);
    let offset = (raw << 8) >> 8;
    (at as i64 + 4 + offset as i64) as usize
}

/// Every body opens with `getlocal_0; pushscope`; a trailing `return` is not
/// followed by an unreachable `returnvoid`; locals follow the parameters.
#[test]
fn straight_line_bodies_encode_exactly() {
    let (body, params, ret, _) = method_in("public function f():void { }", "f");
    assert_eq!(body.code, [0xD0, 0x30, 0x47]);
    assert_eq!((body.max_stack, body.local_count), (1, 1));
    assert!(params.is_empty());
    assert_eq!(ret, "void");

    let (body, params, ret, _) =
        method_in("public function f(a:int, b:String):int { return a; }", "f");
    assert_eq!(body.code, [0xD0, 0x30, 0xD1, 0x48]);
    assert_eq!(body.local_count, 3, "`this` plus two parameters");
    assert_eq!(params, ["int", "String"]);
    assert_eq!(ret, "int");

    let cases: Vec<(String, Vec<u8>)> = vec![
        (
            "public function f():int { return 7; }".into(),
            vec![0xD0, 0x30, 0x24, 0x07, 0x48],
        ),
        (
            "public function f(a:int):void { var x:int = 5; var y:int = 6; }".into(),
            vec![0xD0, 0x30, 0x24, 0x05, 0x63, 0x02, 0x24, 0x06, 0x63, 0x03, 0x47],
        ),
        // AVM2 has no `notEquals`, so `!=` is the equality opcode plus `not`.
        (
            "public function f(a:int, b:int):Boolean { return a != b; }".into(),
            vec![0xD0, 0x30, 0xD1, 0xD2, 0xAB, 0x96, 0x48],
        ),
        (
            "public function f(a:int, b:int):Boolean { return a !== b; }".into(),
            vec![0xD0, 0x30, 0xD1, 0xD2, 0xAC, 0x96, 0x48],
        ),
    ]
    .into_iter()
    .chain(
        [
            ("a + b", 0xA0u8),
            ("a - b", 0xA1),
            ("a * b", 0xA2),
            ("a / b", 0xA3),
            ("a % b", 0xA4),
            ("a < b", 0xAD),
            ("a <= b", 0xAE),
            ("a > b", 0xAF),
            ("a >= b", 0xB0),
        ]
        .map(|(expr, op)| {
            (
                format!("public function f(a:int, b:int):* {{ return {expr}; }}"),
                vec![0xD0, 0x30, 0xD1, 0xD2, op, 0x48],
            )
        }),
    )
    .collect();
    for (src, code) in cases {
        assert_eq!(method(&src).code, code, "for `{src}`");
    }
    assert_eq!(
        method("public function f(a:int):void { var x:int = 5; var y:int = 6; }").local_count,
        4
    );
}

/// `callpropvoid` vs `callproperty` differ in whether a result is left on the
/// stack; picking the wrong one corrupts the operand stack for everything after.
#[test]
fn calls_and_literals_pick_the_right_opcodes() {
    let body = method("public function f():void { this.g(); } public function g():void { }");
    assert_eq!(body.code[..3], [0xD0, 0x30, 0xD0]);
    assert_eq!((body.code[3], body.code[5]), (0x4F, 0x00), "callpropvoid, 0 args");
    assert_eq!(*body.code.last().unwrap(), 0x47);
    assert_eq!(body.max_stack, 1);

    let body = method(
        "public function f():int { return this.g(); } public function g():int { return 1; }",
    );
    assert_eq!(body.code[3], 0x46, "callproperty");
    assert_eq!(*body.code.last().unwrap(), 0x48);

    // Not a member and not a local: left to the scope chain.
    let body = method("public function f():void { trace(1); }");
    assert_eq!(body.code[2], 0x5D, "findpropstrict");
    assert_eq!((body.code[4], body.code[5]), (0x24, 0x01));
    assert_eq!((body.code[6], body.code[8]), (0x4F, 0x01), "callpropvoid, 1 arg");

    assert_eq!(method(r#"public function f():String { return "hello"; }"#).code[2], 0x2C);
    let small = method("public function f():int { return 100; }");
    assert_eq!((small.code[2], small.code[3]), (0x24, 100), "pushbyte");
    assert_eq!(method("public function f():int { return 5000; }").code[2], 0x2D, "pushint");
}

#[test]
fn branches_land_on_the_right_instructions() {
    let c = method(
        "public function f(a:int):void { if (a == 1) { this.g(); } else { this.h(); } } \
         public function g():void { } public function h():void { }",
    )
    .code;
    assert_eq!(c[..3], [0xD0, 0x30, 0xD1]);
    assert_eq!((c[3], c[5], c[6]), (0x24, 0xAB, 0x12), "pushbyte; equals; iffalse");
    let else_at = branch_target(&c, 6);
    assert_eq!((c[else_at], c[else_at + 1]), (0xD0, 0x4F), "else arm calls h on this");
    let jump_at = else_at - 4;
    assert_eq!(c[jump_at], 0x10, "then arm jumps over else");
    let end = branch_target(&c, jump_at);
    assert_eq!((c[end], end), (0x47, c.len() - 1), "arms converge on returnvoid");

    let c = method(
        "public function f(a:int):void { while (a < 3) { this.g(); } } \
         public function g():void { }",
    )
    .code;
    assert_eq!(c[2], 0x09, "backward branches require an AVM2 label");
    assert_eq!(c[3..8], [0xD1, 0x24, 0x03, 0xAD, 0x12]);
    let exit = branch_target(&c, 7);
    assert_eq!(c[exit], 0x47, "the loop exits to returnvoid");
    let back_at = exit - 4;
    assert_eq!(c[back_at], 0x10, "jump");
    assert_eq!(branch_target(&c, back_at), 2, "back to the condition");

    // `&&` must not evaluate its right operand when the left is false.
    let body = method("public function f(a:Boolean, b:Boolean):Boolean { return a && b; }");
    let c = &body.code;
    assert_eq!(c[..5], [0xD0, 0x30, 0xD1, 0x2A, 0x12], "getlocal_1; dup; iffalse");
    assert_eq!((c[8], c[9]), (0x29, 0xD2), "pop; getlocal_2");
    assert_eq!(branch_target(c, 4), 10, "short circuit skips straight to the result");
    assert_eq!(c[10], 0x48);
    assert_eq!(body.max_stack, 2);
}

/// A constructor chains to its base before anything else, whether or not the
/// source says so.
#[test]
fn a_constructor_chains_to_super_then_runs_its_body() {
    let src = "package { import flash.display.MovieClip; \
               public class Foo extends MovieClip { \
               public function Foo() { this.gotoAndStop(1); } } }";
    let abc = compile_source(src).unwrap();
    let detail = parse_abc_detail(DO_ABC_DEFINE, &do_abc_define_body(&abc)).unwrap();
    let class = detail.class("Foo").unwrap();
    let body = detail.body(class.iinit).unwrap();

    assert_eq!(body.code[..5], [0xD0, 0x30, 0xD0, 0x49, 0x00], "constructsuper 0");
    assert_eq!(body.code[5..8], [0xD0, 0x24, 0x01]);
    assert_eq!((body.code[8], body.code[10]), (0x4F, 0x01));
    assert_eq!(*body.code.last().unwrap(), 0x47);
}

/// `a[i]` is a `getproperty`/`setproperty` on a runtime-qualified multiname.
/// HUDFramework's `SendMessage` delivers arguments through `params`, so a widget
/// that cannot index `params` only ever receives a bare signal.
#[test]
fn indexing_uses_a_runtime_qualified_multiname() {
    let (body, _, _, tag) =
        method_in("public function f(params:Array):* { return params[0]; }", "f");
    assert_eq!(body.code[..6], [0xD0, 0x30, 0xD1, 0x24, 0x00, 0x66], "getproperty");
    assert_eq!(*body.code.last().unwrap(), 0x48);
    assert_eq!(body.max_stack, 2);
    let multinames = swf_native::abc::parse_abc_multinames(DO_ABC_DEFINE, &tag).unwrap();
    let mn = &multinames[body.code[6] as usize];
    assert_eq!(mn.kind, 0x1B, "MultinameL, not a QName");
    assert_eq!(mn.name, "", "the name comes off the stack");
    assert_eq!(mn.namespaces, [(0x16u8, String::new())]);

    let body = method("public function f(a:Array):void { a[1] = 7; }");
    let ops = swf_native::abc_edit::decode(&body.code).unwrap();
    use swf::avm2::types::Op;
    let count = |pred: &dyn Fn(&Op) -> bool| ops.iter().filter(|(_, op)| pred(op)).count();
    assert_eq!(count(&|op| matches!(op, Op::SetProperty { .. })), 1);
    assert_eq!(count(&|op| matches!(op, Op::GetLocal { index: 1 })), 1);
    assert_eq!(count(&|op| *op == Op::PushByte { value: 1 }), 1);
    assert_eq!(count(&|op| *op == Op::PushByte { value: 7 }), 1);
    assert_eq!(body.max_stack, 3, "the store consumes object, index and value");

    let body = method(
        "public function f(params:Array):void { this.g(params[0]); } \
         public function g(v:*):void { }",
    );
    assert_eq!(body.code[..7], [0xD0, 0x30, 0xD0, 0xD1, 0x24, 0x00, 0x66]);
    assert_eq!((body.code[8], body.code[10]), (0x4F, 0x01));
    assert_eq!(body.max_stack, 3);
}
