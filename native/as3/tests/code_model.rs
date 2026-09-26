//! Branch fixups and the stack/scope analysis.
//!
//! No AVM2 runtime is available and `swf_native::abc` does not decode method
//! bodies, so these bytes are asserted against the AVM2 Overview directly: a
//! branch's `s24` operand is signed, little-endian and relative to the byte
//! after the operand, so a branch to the next instruction encodes 0.

use as3_native::abc::code::{CodeBuilder, Op};

#[test]
fn branch_offsets_encode_relative_s24_over_encoded_bytes() {
    let mut code = CodeBuilder::new();
    let done = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(done))
        .place(done)
        .emit(Op::ReturnVoid);
    assert_eq!(
        code.assemble().unwrap(),
        [0x26, 0x12, 0x00, 0x00, 0x00, 0x47]
    );

    // 8 - (1 + 4) == 3: the three bytes of pushbyte+pop.
    let mut code = CodeBuilder::new();
    let done = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(done))
        .emit(Op::PushByte(1))
        .emit(Op::Pop)
        .place(done)
        .emit(Op::ReturnVoid);
    assert_eq!(
        code.assemble().unwrap(),
        [0x26, 0x12, 0x03, 0x00, 0x00, 0x24, 0x01, 0x29, 0x47]
    );

    // 0 - (1 + 4) == -5, two's complement in three little-endian bytes.
    let mut code = CodeBuilder::new();
    let top = code.new_label();
    code.place(top)
        .emit(Op::PushTrue)
        .emit(Op::IfFalse(top))
        .emit(Op::ReturnVoid);
    assert_eq!(
        code.assemble().unwrap(),
        [0x26, 0x12, 0xFB, 0xFF, 0xFF, 0x47]
    );

    // getlex 200 needs a two-byte u30, so the skip is 3 + 1 bytes, not 2 instructions.
    let mut code = CodeBuilder::new();
    let done = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(done))
        .emit(Op::GetLex(200))
        .emit(Op::Pop)
        .place(done)
        .emit(Op::ReturnVoid);
    let bytes = code.assemble().unwrap();
    assert_eq!(&bytes[1..5], &[0x12, 0x04, 0x00, 0x00]);
    assert_eq!(&bytes[5..9], &[0x60, 0xC8, 0x01, 0x29]);
}

#[test]
fn analysis_computes_stack_scope_and_local_counts() {
    // Constructor shape: starts at scope depth 4, the single pushscope reaches 5.
    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0)
        .emit(Op::PushScope)
        .emit(Op::GetLocal0)
        .emit(Op::ConstructSuper(0))
        .emit(Op::ReturnVoid);
    let stats = code.analyze(4, 1).unwrap();
    assert_eq!(
        (stats.max_stack, stats.local_count, stats.max_scope_depth),
        (1, 1, 5)
    );

    // Script initialiser shape.
    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0)
        .emit(Op::PushScope)
        .emit(Op::GetScopeObject(0))
        .emit(Op::GetLex(1))
        .emit(Op::PushScope)
        .emit(Op::GetLex(1))
        .emit(Op::NewClass(0))
        .emit(Op::PopScope)
        .emit(Op::InitProperty(2))
        .emit(Op::ReturnVoid);
    let stats = code.analyze(1, 1).unwrap();
    assert_eq!((stats.max_stack, stats.max_scope_depth), (2, 3));

    // Both paths reach the join empty, but max_stack saw the taken path's peak.
    let mut code = CodeBuilder::new();
    let join = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(join))
        .emit(Op::PushByte(1))
        .emit(Op::PushByte(2))
        .emit(Op::Pop)
        .emit(Op::Pop)
        .place(join)
        .emit(Op::ReturnVoid);
    assert_eq!(code.analyze(0, 1).unwrap().max_stack, 2);

    // A loop back edge re-enters an instruction that already has a depth.
    let mut code = CodeBuilder::new();
    let top = code.new_label();
    code.place(top)
        .emit(Op::PushTrue)
        .emit(Op::IfTrue(top))
        .emit(Op::ReturnVoid);
    assert_eq!(code.analyze(0, 1).unwrap().max_stack, 1);

    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0)
        .emit(Op::SetLocal(5))
        .emit(Op::ReturnVoid);
    assert_eq!(code.analyze(0, 1).unwrap().local_count, 6);
    // A declared minimum still wins when no high register is used.
    let mut plain = CodeBuilder::new();
    plain.emit(Op::ReturnVoid);
    assert_eq!(plain.analyze(0, 3).unwrap().local_count, 3);
}

#[test]
fn invalid_bodies_are_reported() {
    let mut code = CodeBuilder::new();
    let dangling = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(dangling))
        .emit(Op::ReturnVoid);
    let err = code.assemble().unwrap_err();
    assert!(err.contains("never placed"), "{err}");

    let mut code = CodeBuilder::new();
    let join = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(join))
        .emit(Op::PushByte(1))
        .place(join)
        .emit(Op::ReturnVoid);
    let err = code.analyze(0, 1).unwrap_err();
    assert!(err.contains("inconsistent depth"), "{err}");

    let mut code = CodeBuilder::new();
    code.emit(Op::Pop).emit(Op::ReturnVoid);
    let err = code.analyze(0, 1).unwrap_err();
    assert!(err.contains("underflow"), "{err}");

    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0).emit(Op::PushScope);
    let err = code.analyze(1, 1).unwrap_err();
    assert!(err.contains("without a return"), "{err}");
}
