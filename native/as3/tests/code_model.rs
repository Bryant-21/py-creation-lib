//! Branch fixups and the stack/scope analysis.
//!
//! No AVM2 runtime is available and `swf_native::abc` does not decode method
//! bodies, so these bytes are asserted against the AVM2 Overview directly: a
//! branch's `s24` operand is signed, little-endian and relative to the byte
//! after the operand, so a branch to the next instruction encodes 0.

use as3_native::abc::code::{CodeBuilder, Op};

#[test]
fn a_branch_to_the_next_instruction_encodes_zero() {
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
}

#[test]
fn a_forward_branch_skips_exactly_the_intervening_bytes() {
    let mut code = CodeBuilder::new();
    let done = code.new_label();
    code.emit(Op::PushTrue) // offset 0, 1 byte
        .emit(Op::IfFalse(done)) // offset 1, 4 bytes
        .emit(Op::PushByte(1)) // offset 5, 2 bytes
        .emit(Op::Pop) // offset 7, 1 byte
        .place(done) // offset 8
        .emit(Op::ReturnVoid);

    // 8 - (1 + 4) == 3: the three bytes of pushbyte+pop.
    assert_eq!(
        code.assemble().unwrap(),
        [0x26, 0x12, 0x03, 0x00, 0x00, 0x24, 0x01, 0x29, 0x47]
    );
}

#[test]
fn a_backward_branch_encodes_a_negative_offset() {
    let mut code = CodeBuilder::new();
    let top = code.new_label();
    code.place(top) // offset 0
        .emit(Op::PushTrue) // offset 0, 1 byte
        .emit(Op::IfFalse(top)) // offset 1, 4 bytes
        .emit(Op::ReturnVoid);

    // 0 - (1 + 4) == -5, two's complement in three little-endian bytes.
    assert_eq!(
        code.assemble().unwrap(),
        [0x26, 0x12, 0xFB, 0xFF, 0xFF, 0x47]
    );
}

/// Operand widths are variable, so a branch over a multi-byte operand has to
/// count the encoded bytes rather than the instruction count.
#[test]
fn branch_offsets_count_encoded_operand_bytes_not_instructions() {
    let mut code = CodeBuilder::new();
    let done = code.new_label();
    code.emit(Op::PushTrue) // 1 byte
        .emit(Op::IfFalse(done)) // 4 bytes, at offset 1
        // A multiname index of 200 needs a two-byte u30, so this is 3 bytes.
        .emit(Op::GetLex(200))
        .emit(Op::Pop) // 1 byte
        .place(done)
        .emit(Op::ReturnVoid);

    let bytes = code.assemble().unwrap();
    assert_eq!(&bytes[1..5], &[0x12, 0x04, 0x00, 0x00]);
    // getlex 200 == 0xC8 0x01 in u30, proving the 4 above is 3 + 1 and not 2.
    assert_eq!(&bytes[5..9], &[0x60, 0xC8, 0x01, 0x29]);
}

#[test]
fn an_unplaced_label_is_rejected() {
    let mut code = CodeBuilder::new();
    let dangling = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(dangling))
        .emit(Op::ReturnVoid);

    let err = code.assemble().unwrap_err();
    assert!(err.contains("never placed"), "{err}");
}

#[test]
fn depths_are_computed_for_the_constructor_shape() {
    // getlocal_0; pushscope; getlocal_0; constructsuper 0; returnvoid
    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0)
        .emit(Op::PushScope)
        .emit(Op::GetLocal0)
        .emit(Op::ConstructSuper(0))
        .emit(Op::ReturnVoid);

    let stats = code.analyze(4, 1).unwrap();
    assert_eq!(stats.max_stack, 1);
    assert_eq!(stats.local_count, 1);
    // Starts at 4, the single pushscope reaches 5.
    assert_eq!(stats.max_scope_depth, 5);
}

#[test]
fn depths_are_computed_for_the_script_initialiser_shape() {
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
    assert_eq!(stats.max_stack, 2);
    assert_eq!(stats.max_scope_depth, 3);
}

#[test]
fn both_paths_into_a_join_must_agree_and_the_deeper_one_sets_max_stack() {
    let mut code = CodeBuilder::new();
    let join = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(join))
        // Taken path pushes two values and drops them again, so the join is
        // still reached with an empty stack — but max_stack saw the peak.
        .emit(Op::PushByte(1))
        .emit(Op::PushByte(2))
        .emit(Op::Pop)
        .emit(Op::Pop)
        .place(join)
        .emit(Op::ReturnVoid);

    let stats = code.analyze(0, 1).unwrap();
    assert_eq!(stats.max_stack, 2);
}

#[test]
fn disagreeing_stack_depths_at_a_join_are_reported() {
    let mut code = CodeBuilder::new();
    let join = code.new_label();
    code.emit(Op::PushTrue)
        .emit(Op::IfFalse(join))
        // Falls into the join with one extra value on the stack.
        .emit(Op::PushByte(1))
        .place(join)
        .emit(Op::ReturnVoid);

    let err = code.analyze(0, 1).unwrap_err();
    assert!(err.contains("inconsistent depth"), "{err}");
}

#[test]
fn a_stack_underflow_is_reported() {
    let mut code = CodeBuilder::new();
    code.emit(Op::Pop).emit(Op::ReturnVoid);
    let err = code.analyze(0, 1).unwrap_err();
    assert!(err.contains("underflow"), "{err}");
}

#[test]
fn falling_off_the_end_of_a_body_is_reported() {
    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0).emit(Op::PushScope);
    let err = code.analyze(1, 1).unwrap_err();
    assert!(err.contains("without a return"), "{err}");
}

#[test]
fn local_count_follows_the_highest_register_touched() {
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

/// A loop is the case a linear scan would get wrong: the back edge re-enters an
/// instruction that has already been assigned a depth.
#[test]
fn a_loop_back_edge_is_analyzed_once_and_verified() {
    let mut code = CodeBuilder::new();
    let top = code.new_label();
    code.place(top)
        .emit(Op::PushTrue)
        .emit(Op::IfTrue(top))
        .emit(Op::ReturnVoid);

    let stats = code.analyze(0, 1).unwrap();
    assert_eq!(stats.max_stack, 1);
}
