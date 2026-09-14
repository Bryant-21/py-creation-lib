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
/// `method_info` signature.
fn method(members: &str, name: &str) -> (AbcBody, Vec<String>, String) {
    let src = format!("package {{ public class Foo {{ {members} }} }}");
    let abc = compile_source(&src).unwrap_or_else(|e| panic!("compiling {src}: {e}"));
    let detail = parse_abc_detail(DO_ABC_DEFINE, &do_abc_define_body(&abc)).unwrap();
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
    )
}

/// Decode the `s24` operand of the branch at `at`, returning the byte offset it
/// targets.
fn branch_target(code: &[u8], at: usize) -> usize {
    let raw = i32::from_le_bytes([code[at + 1], code[at + 2], code[at + 3], 0]);
    // Sign-extend from 24 bits.
    let offset = (raw << 8) >> 8;
    (at as i64 + 4 + offset as i64) as usize
}

/// Every method body opens by establishing `this` as its scope. An empty
/// `void` method is exactly that plus a return.
#[test]
fn an_empty_method_is_prologue_plus_return() {
    let (body, params, ret) = method("public function f():void { }", "f");
    assert_eq!(
        body.code,
        [0xD0, 0x30, 0x47],
        "getlocal_0; pushscope; returnvoid"
    );
    assert_eq!(body.max_stack, 1);
    assert_eq!(body.local_count, 1, "`this` only");
    assert!(params.is_empty());
    assert_eq!(ret, "void");
}

#[test]
fn parameters_become_registers_and_are_declared_in_the_signature() {
    let (body, params, ret) = method("public function f(a:int, b:String):int { return a; }", "f");
    // getlocal_0; pushscope; getlocal_1; returnvalue
    assert_eq!(body.code, [0xD0, 0x30, 0xD1, 0x48]);
    assert_eq!(body.local_count, 3, "`this` plus two parameters");
    assert_eq!(params, ["int", "String"]);
    assert_eq!(ret, "int");
}

/// A `return` at the end of a body must not be followed by a second, unreachable
/// `returnvoid`.
#[test]
fn a_trailing_return_is_not_doubled() {
    let (body, _, _) = method("public function f():int { return 7; }", "f");
    assert_eq!(body.code, [0xD0, 0x30, 0x24, 0x07, 0x48]);
    assert_eq!(
        *body.code.last().unwrap(),
        0x48,
        "returnvalue, not returnvoid"
    );
}

#[test]
fn a_call_on_this_uses_callpropvoid_when_the_result_is_discarded() {
    let (body, _, _) = method(
        "public function f():void { this.g(); } public function g():void { }",
        "f",
    );
    // getlocal_0; pushscope; getlocal_0; callpropvoid <g> 0; returnvoid
    assert_eq!(body.code[..3], [0xD0, 0x30, 0xD0]);
    assert_eq!(body.code[3], 0x4F, "callpropvoid");
    assert_eq!(body.code[5], 0x00, "zero arguments");
    assert_eq!(*body.code.last().unwrap(), 0x47);
    assert_eq!(body.max_stack, 1);
}

/// A call whose value *is* used must be `callproperty`, not `callpropvoid` —
/// the two differ in whether a result is left on the stack, and picking the
/// wrong one corrupts the operand stack for everything after it.
#[test]
fn a_call_whose_value_is_used_uses_callproperty() {
    let (body, _, _) = method(
        "public function f():int { return this.g(); } public function g():int { return 1; }",
        "f",
    );
    assert_eq!(body.code[3], 0x46, "callproperty");
    assert_eq!(*body.code.last().unwrap(), 0x48, "returnvalue");
}

/// An unqualified name that is not a member and not a local is left to the
/// scope chain.
#[test]
fn an_unqualified_call_uses_findpropstrict() {
    let (body, _, _) = method("public function f():void { trace(1); }", "f");
    assert_eq!(body.code[2], 0x5D, "findpropstrict");
    // findpropstrict <mn>; pushbyte 1; callpropvoid <mn> 1
    assert_eq!(body.code[4], 0x24);
    assert_eq!(body.code[5], 0x01);
    assert_eq!(body.code[6], 0x4F);
    assert_eq!(body.code[8], 0x01, "one argument");
}

#[test]
fn locals_are_allocated_after_the_parameters() {
    let (body, _, _) = method(
        "public function f(a:int):void { var x:int = 5; var y:int = 6; }",
        "f",
    );
    // getlocal_0; pushscope; pushbyte 5; setlocal_2; pushbyte 6; setlocal_3
    assert_eq!(
        body.code,
        [
            0xD0, 0x30, 0x24, 0x05, 0x63, 0x02, 0x24, 0x06, 0x63, 0x03, 0x47
        ]
    );
    assert_eq!(body.local_count, 4, "this, a, x, y");
}

#[test]
fn if_else_branches_land_on_the_right_instructions() {
    let (body, _, _) = method(
        "public function f(a:int):void { if (a == 1) { this.g(); } else { this.h(); } } \
         public function g():void { } public function h():void { }",
        "f",
    );
    let c = &body.code;
    // getlocal_0; pushscope; getlocal_1; pushbyte 1; equals; iffalse ...
    assert_eq!(c[..3], [0xD0, 0x30, 0xD1]);
    assert_eq!(c[3], 0x24);
    assert_eq!(c[5], 0xAB, "equals");
    assert_eq!(c[6], 0x12, "iffalse");

    // The false branch must land on the `else` arm, which begins with the
    // getlocal_0 that pushes the receiver.
    let else_at = branch_target(c, 6);
    assert_eq!(c[else_at], 0xD0, "else arm starts by pushing `this`");
    assert_eq!(c[else_at + 1], 0x4F, "and calls h");

    // The `then` arm ends in a jump over the `else` arm, to the returnvoid.
    let jump_at = else_at - 4;
    assert_eq!(c[jump_at], 0x10, "jump");
    let end = branch_target(c, jump_at);
    assert_eq!(c[end], 0x47, "both arms converge on returnvoid");
    assert_eq!(end, c.len() - 1);
}

#[test]
fn a_while_loop_branches_forward_to_the_exit_and_back_to_the_test() {
    let (body, _, _) = method(
        "public function f(a:int):void { while (a < 3) { this.g(); } } \
         public function g():void { }",
        "f",
    );
    let c = &body.code;
    // The loop test starts right after the prologue.
    assert_eq!(c[..2], [0xD0, 0x30]);
    // getlocal_1; pushbyte 3; lessthan; iffalse
    assert_eq!(c[2], 0xD1, "getlocal_1");
    assert_eq!((c[3], c[4]), (0x24, 0x03), "pushbyte 3");
    assert_eq!(c[5], 0xAD, "lessthan");
    assert_eq!(c[6], 0x12, "iffalse");

    let exit = branch_target(c, 6);
    assert_eq!(c[exit], 0x47, "the loop exits to returnvoid");

    // The body ends with a backward jump to the test.
    let back_at = exit - 4;
    assert_eq!(c[back_at], 0x10, "jump");
    assert_eq!(branch_target(c, back_at), 2, "back to the condition");
    // A backward branch really is negative.
    assert!(i32::from_le_bytes([c[back_at + 1], c[back_at + 2], c[back_at + 3], 0]) << 8 < 0);
}

/// `&&` must not evaluate its right operand when the left is false, so it
/// branches rather than reducing to an opcode.
#[test]
fn logical_and_short_circuits() {
    let (body, _, _) = method(
        "public function f(a:Boolean, b:Boolean):Boolean { return a && b; }",
        "f",
    );
    let c = &body.code;
    // getlocal_0; pushscope; getlocal_1; dup; iffalse end; pop; getlocal_2; returnvalue
    assert_eq!(c[..4], [0xD0, 0x30, 0xD1, 0x2A]);
    assert_eq!(c[4], 0x12, "iffalse");
    assert_eq!(c[8], 0x29, "pop the duplicated false value");
    assert_eq!(c[9], 0xD2, "getlocal_2");
    let end = branch_target(c, 4);
    assert_eq!(end, 10, "short circuit skips straight to the result");
    assert_eq!(c[end], 0x48, "returnvalue");
    // Both paths leave exactly one value.
    assert_eq!(body.max_stack, 2);
}

#[test]
fn comparison_and_arithmetic_use_the_expected_opcodes() {
    for (expr, opcode) in [
        ("a + b", 0xA0u8),
        ("a - b", 0xA1),
        ("a * b", 0xA2),
        ("a / b", 0xA3),
        ("a % b", 0xA4),
        ("a < b", 0xAD),
        ("a <= b", 0xAE),
        ("a > b", 0xAF),
        ("a >= b", 0xB0),
    ] {
        let (body, _, _) = method(
            &format!("public function f(a:int, b:int):* {{ return {expr}; }}"),
            "f",
        );
        assert_eq!(
            body.code,
            [0xD0, 0x30, 0xD1, 0xD2, opcode, 0x48],
            "for `{expr}`"
        );
    }
}

/// AVM2 has no `notEquals`, so `!=` is the equality opcode plus `not`.
#[test]
fn inequality_negates_the_equality_opcode() {
    let (body, _, _) = method(
        "public function f(a:int, b:int):Boolean { return a != b; }",
        "f",
    );
    assert_eq!(body.code, [0xD0, 0x30, 0xD1, 0xD2, 0xAB, 0x96, 0x48]);

    let (strict, _, _) = method(
        "public function f(a:int, b:int):Boolean { return a !== b; }",
        "f",
    );
    assert_eq!(strict.code, [0xD0, 0x30, 0xD1, 0xD2, 0xAC, 0x96, 0x48]);
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

    // getlocal_0; pushscope; getlocal_0; constructsuper 0; then the body.
    assert_eq!(body.code[..5], [0xD0, 0x30, 0xD0, 0x49, 0x00]);
    assert_eq!(body.code[5], 0xD0, "receiver for gotoAndStop");
    assert_eq!((body.code[6], body.code[7]), (0x24, 0x01), "pushbyte 1");
    assert_eq!(body.code[8], 0x4F, "callpropvoid");
    assert_eq!(body.code[10], 0x01, "one argument");
    assert_eq!(*body.code.last().unwrap(), 0x47);
}

/// String literals reach the constant pool and are pushed by index.
#[test]
fn string_literals_are_pooled_and_pushed() {
    let (body, _, _) = method(r#"public function f():String { return "hello"; }"#, "f");
    assert_eq!(body.code[2], 0x2C, "pushstring");
    assert_eq!(*body.code.last().unwrap(), 0x48);
}

/// Larger integers do not fit `pushbyte` and must go through the int pool.
#[test]
fn integers_choose_pushbyte_or_the_int_pool_by_size() {
    let (small, _, _) = method("public function f():int { return 100; }", "f");
    assert_eq!(small.code[2], 0x24, "pushbyte");
    assert_eq!(small.code[3], 100);

    let (large, _, _) = method("public function f():int { return 5000; }", "f");
    assert_eq!(large.code[2], 0x2D, "pushint");
}

/// `a[i]` compiles to a `getproperty` on a runtime-qualified multiname: the
/// index is pushed and consumed as the property name. The wrong operand kind
/// leaves the stack one deep on every access.
///
/// HUDFramework's `SendMessage` delivers arguments through `params`, so a widget
/// that cannot index `params` only ever receives a bare signal.
#[test]
fn indexing_uses_a_runtime_qualified_multiname() {
    let src = "package { public class Foo { \
               public function f(params:Array):* { return params[0]; } } }";
    let abc = compile_source(src).unwrap();
    let body_tag = do_abc_define_body(&abc);
    let detail = parse_abc_detail(DO_ABC_DEFINE, &body_tag).unwrap();
    let class = detail.class("Foo").unwrap();
    let t = class
        .instance_traits
        .iter()
        .find(|t| t.name == "f")
        .unwrap();
    let body = detail.body(t.index).unwrap();

    // getlocal_0; pushscope; getlocal_1; pushbyte 0; getproperty <L>; returnvalue
    assert_eq!(body.code[..3], [0xD0, 0x30, 0xD1]);
    assert_eq!((body.code[3], body.code[4]), (0x24, 0x00), "pushbyte 0");
    assert_eq!(body.code[5], 0x66, "getproperty");
    let operand = body.code[6] as u32;
    assert_eq!(*body.code.last().unwrap(), 0x48, "returnvalue");

    let multinames = swf_native::abc::parse_abc_multinames(DO_ABC_DEFINE, &body_tag).unwrap();
    let mn = &multinames[operand as usize];
    assert_eq!(mn.kind, 0x1B, "MultinameL, not a QName");
    assert_eq!(mn.name, "", "the name comes off the stack");
    assert_eq!(
        mn.namespaces,
        [(0x16u8, String::new())],
        "looked up in the public namespace"
    );

    // Object and index in, one value out: the net effect must be a single push.
    assert_eq!(body.max_stack, 2);
}

#[test]
fn an_indexed_store_pops_object_index_and_value() {
    let src = "package { public class Foo { \
               public function f(a:Array):void { a[1] = 7; } } }";
    let abc = compile_source(src).unwrap();
    let detail = parse_abc_detail(DO_ABC_DEFINE, &do_abc_define_body(&abc)).unwrap();
    let class = detail.class("Foo").unwrap();
    let t = class
        .instance_traits
        .iter()
        .find(|t| t.name == "f")
        .unwrap();
    let body = detail.body(t.index).unwrap();

    // getlocal_0; pushscope; getlocal_1; pushbyte 1; pushbyte 7; setproperty <L>
    assert_eq!(body.code[..3], [0xD0, 0x30, 0xD1]);
    assert_eq!((body.code[3], body.code[4]), (0x24, 0x01));
    assert_eq!((body.code[5], body.code[6]), (0x24, 0x07));
    assert_eq!(body.code[7], 0x61, "setproperty");
    assert_eq!(*body.code.last().unwrap(), 0x47, "returnvoid");
    // Three operands live at the peak, and the store consumes all of them.
    assert_eq!(body.max_stack, 3);
}

/// The payload path end to end: index `params`, and use the value.
#[test]
fn a_widget_can_read_its_message_payload() {
    let src = "package { public class Foo { \
               public function f(params:Array):void { this.g(params[0]); } \
               public function g(v:*):void { } } }";
    let abc = compile_source(src).unwrap();
    let detail = parse_abc_detail(DO_ABC_DEFINE, &do_abc_define_body(&abc)).unwrap();
    let class = detail.class("Foo").unwrap();
    let t = class
        .instance_traits
        .iter()
        .find(|t| t.name == "f")
        .unwrap();
    let body = detail.body(t.index).unwrap();

    // getlocal_0 (receiver); getlocal_1 (params); pushbyte 0; getproperty <L>;
    // callpropvoid g 1
    assert_eq!(body.code[..4], [0xD0, 0x30, 0xD0, 0xD1]);
    assert_eq!((body.code[4], body.code[5]), (0x24, 0x00));
    assert_eq!(body.code[6], 0x66);
    assert_eq!(body.code[8], 0x4F, "callpropvoid");
    assert_eq!(body.code[10], 0x01, "one argument");
    assert_eq!(body.max_stack, 3);
}
