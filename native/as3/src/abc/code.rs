//! Instruction list, branch fixups, and the stack/scope analysis that fills in
//! a method body's `max_stack`, `local_count` and `max_scope_depth`.
//!
//! Written from the *AVM2 Overview* opcode table and verifier rules (section
//! 2.4), not from another compiler's source.
//!
//! Branches target symbolic [`Label`]s resolved after layout, because an `s24`
//! operand is relative to the byte after it. Each op's width depends only on its
//! own operands, so one sizing pass suffices. [`analyze`] computes the depth
//! fields by walking the control-flow graph; two paths that reach an instruction
//! at different depths are reported as a compiler bug instead of being emitted
//! for the player's verifier to reject.

use super::pool::write_u30;

/// A symbolic branch target. Created by [`CodeBuilder::new_label`] and given a
/// position by [`CodeBuilder::place`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Label(usize);

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Nop,
    Label,
    BitAnd,
    BitOr,
    BitXor,
    LShift,
    RShift,
    URShift,
    BitNot,
    TypeOf,
    IsTypeLate,
    AsTypeLate,
    InstanceOf,
    In,
    NextName,
    NextValue,
    Throw,
    Construct(u32),
    NewArray(u32),
    NewObject(u32),
    ApplyType(u32),
    Call(u32),
    HasNext2(u32, u32),
    Pop,
    Dup,

    GetLocal0,
    GetLocal1,
    GetLocal2,
    GetLocal3,
    GetLocal(u32),
    SetLocal(u32),

    PushNull,
    PushTrue,
    PushFalse,
    PushByte(i8),
    PushInt(u32),
    PushUInt(u32),
    PushDouble(u32),
    PushString(u32),

    PushScope,
    PopScope,
    GetScopeObject(u8),
    GetGlobalScope,

    GetLex(u32),
    FindPropStrict(u32),
    GetProperty(u32),
    SetProperty(u32),
    InitProperty(u32),
    /// `getproperty` whose operand is a `MultinameL`: the property name is
    /// taken off the stack, so this pops the index as well as the object.
    /// Separate from [`Op::GetProperty`] precisely so the stack accounting
    /// cannot silently be wrong.
    GetPropertyIndexed(u32),
    /// `setproperty` with a `MultinameL`: pops object, index and value.
    SetPropertyIndexed(u32),
    CallProperty {
        name: u32,
        arg_count: u32,
    },
    CallPropVoid {
        name: u32,
        arg_count: u32,
    },

    ConstructSuper(u32),
    NewClass(u32),

    /// Coerce to the named type. `CoerceAny` is the `*` type.
    Coerce(u32),
    CoerceAny,

    Negate,
    Not,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equals,
    StrictEquals,
    LessThan,
    LessEquals,
    GreaterThan,
    GreaterEquals,

    ReturnVoid,
    ReturnValue,

    Jump(Label),
    IfTrue(Label),
    IfFalse(Label),
    IfEq(Label),
    IfNe(Label),
    IfStrictEq(Label),
    IfStrictNe(Label),
    IfLt(Label),
    IfLe(Label),
    IfGt(Label),
    IfGe(Label),
}

impl Op {
    fn opcode(&self) -> u8 {
        match self {
            Op::Nop => 0x02,
            Op::Label => 0x09,
            Op::BitAnd => 0xA8,
            Op::BitOr => 0xA9,
            Op::BitXor => 0xAA,
            Op::LShift => 0xA5,
            Op::RShift => 0xA6,
            Op::URShift => 0xA7,
            Op::BitNot => 0x97,
            Op::TypeOf => 0x95,
            Op::IsTypeLate => 0xB3,
            Op::AsTypeLate => 0x87,
            Op::InstanceOf => 0xB1,
            Op::In => 0xB4,
            Op::NextName => 0x1E,
            Op::NextValue => 0x23,
            Op::Throw => 0x03,
            Op::Construct(_) => 0x42,
            Op::NewArray(_) => 0x56,
            Op::NewObject(_) => 0x55,
            Op::ApplyType(_) => 0x53,
            Op::Call(_) => 0x41,
            Op::HasNext2(..) => 0x32,
            Op::Pop => 0x29,
            Op::Dup => 0x2A,
            Op::GetLocal0 => 0xD0,
            Op::GetLocal1 => 0xD1,
            Op::GetLocal2 => 0xD2,
            Op::GetLocal3 => 0xD3,
            Op::GetLocal(_) => 0x62,
            Op::SetLocal(_) => 0x63,
            Op::PushNull => 0x20,
            Op::PushTrue => 0x26,
            Op::PushFalse => 0x27,
            Op::PushByte(_) => 0x24,
            Op::PushInt(_) => 0x2D,
            Op::PushUInt(_) => 0x2E,
            Op::PushDouble(_) => 0x2F,
            Op::PushString(_) => 0x2C,
            Op::PushScope => 0x30,
            Op::PopScope => 0x1D,
            Op::GetScopeObject(_) => 0x65,
            Op::GetGlobalScope => 0x64,
            Op::GetLex(_) => 0x60,
            Op::FindPropStrict(_) => 0x5D,
            Op::GetProperty(_) | Op::GetPropertyIndexed(_) => 0x66,
            Op::SetProperty(_) | Op::SetPropertyIndexed(_) => 0x61,
            Op::InitProperty(_) => 0x68,
            Op::CallProperty { .. } => 0x46,
            Op::CallPropVoid { .. } => 0x4F,
            Op::ConstructSuper(_) => 0x49,
            Op::NewClass(_) => 0x58,
            Op::Coerce(_) => 0x80,
            Op::CoerceAny => 0x82,
            Op::Negate => 0x90,
            Op::Not => 0x96,
            Op::Add => 0xA0,
            Op::Subtract => 0xA1,
            Op::Multiply => 0xA2,
            Op::Divide => 0xA3,
            Op::Modulo => 0xA4,
            Op::Equals => 0xAB,
            Op::StrictEquals => 0xAC,
            Op::LessThan => 0xAD,
            Op::LessEquals => 0xAE,
            Op::GreaterThan => 0xAF,
            Op::GreaterEquals => 0xB0,
            Op::ReturnVoid => 0x47,
            Op::ReturnValue => 0x48,
            Op::Jump(_) => 0x10,
            Op::IfTrue(_) => 0x11,
            Op::IfFalse(_) => 0x12,
            Op::IfEq(_) => 0x13,
            Op::IfNe(_) => 0x14,
            Op::IfLt(_) => 0x15,
            Op::IfLe(_) => 0x16,
            Op::IfGt(_) => 0x17,
            Op::IfGe(_) => 0x18,
            Op::IfStrictEq(_) => 0x19,
            Op::IfStrictNe(_) => 0x1A,
        }
    }

    /// Net change in operand-stack depth.
    ///
    /// Indexed properties account for their additional runtime-name operand.
    fn stack_delta(&self) -> i32 {
        match self {
            Op::BitAnd => -1,
            Op::BitOr => -1,
            Op::BitXor => -1,
            Op::LShift => -1,
            Op::RShift => -1,
            Op::URShift => -1,
            Op::BitNot => 0,
            Op::TypeOf => 0,
            Op::IsTypeLate => -1,
            Op::AsTypeLate => -1,
            Op::InstanceOf => -1,
            Op::In => -1,
            Op::NextName => -1,
            Op::NextValue => -1,
            Op::Throw => -1,
            Op::Construct(v) => -(*v as i32),
            Op::NewArray(v) => 1 - *v as i32,
            Op::NewObject(v) => 1 - 2 * *v as i32,
            Op::ApplyType(v) => -(*v as i32),
            Op::Call(v) => -(*v as i32) - 1,
            Op::HasNext2(..) => 1,
            Op::Nop | Op::Label | Op::PopScope | Op::Jump(_) | Op::ReturnVoid => 0,
            // Unary and coercion operators replace the value in place.
            Op::Coerce(_) | Op::CoerceAny | Op::Negate | Op::Not => 0,
            // Binary operators pop both operands and push one result.
            Op::Add
            | Op::Subtract
            | Op::Multiply
            | Op::Divide
            | Op::Modulo
            | Op::Equals
            | Op::StrictEquals
            | Op::LessThan
            | Op::LessEquals
            | Op::GreaterThan
            | Op::GreaterEquals => -1,
            Op::Pop | Op::ReturnValue | Op::SetLocal(_) | Op::PushScope => -1,
            Op::IfTrue(_) | Op::IfFalse(_) => -1,
            Op::IfEq(_)
            | Op::IfNe(_)
            | Op::IfStrictEq(_)
            | Op::IfStrictNe(_)
            | Op::IfLt(_)
            | Op::IfLe(_)
            | Op::IfGt(_)
            | Op::IfGe(_) => -2,
            Op::Dup
            | Op::GetLocal0
            | Op::GetLocal1
            | Op::GetLocal2
            | Op::GetLocal3
            | Op::GetLocal(_)
            | Op::PushNull
            | Op::PushTrue
            | Op::PushFalse
            | Op::PushByte(_)
            | Op::PushInt(_)
            | Op::PushUInt(_)
            | Op::PushDouble(_)
            | Op::PushString(_)
            | Op::GetScopeObject(_)
            | Op::GetGlobalScope
            | Op::GetLex(_)
            | Op::FindPropStrict(_) => 1,
            // Pops the object, pushes the value.
            Op::GetProperty(_) => 0,
            Op::SetProperty(_) | Op::InitProperty(_) => -2,
            // A runtime-qualified name is itself an operand: `a[i]` pops the
            // object and the index, and a store pops the value too.
            Op::GetPropertyIndexed(_) => -1,
            Op::SetPropertyIndexed(_) => -3,
            // Pops the base class, pushes the new class.
            Op::NewClass(_) => 0,
            Op::ConstructSuper(n) => -(*n as i32) - 1,
            Op::CallPropVoid { arg_count, .. } => -(*arg_count as i32) - 1,
            Op::CallProperty { arg_count, .. } => -(*arg_count as i32),
        }
    }

    fn scope_delta(&self) -> i32 {
        match self {
            Op::PushScope => 1,
            Op::PopScope => -1,
            _ => 0,
        }
    }

    /// The local register this instruction reads or writes, if any. Used to
    /// derive `local_count` from observed register use.
    fn register(&self) -> Option<u32> {
        match self {
            Op::HasNext2(a, b) => Some((*a).max(*b)),
            Op::GetLocal0 => Some(0),
            Op::GetLocal1 => Some(1),
            Op::GetLocal2 => Some(2),
            Op::GetLocal3 => Some(3),
            Op::GetLocal(r) | Op::SetLocal(r) => Some(*r),
            _ => None,
        }
    }

    fn branch_target(&self) -> Option<Label> {
        match self {
            Op::Jump(l)
            | Op::IfTrue(l)
            | Op::IfFalse(l)
            | Op::IfEq(l)
            | Op::IfNe(l)
            | Op::IfStrictEq(l)
            | Op::IfStrictNe(l)
            | Op::IfLt(l)
            | Op::IfLe(l)
            | Op::IfGt(l)
            | Op::IfGe(l) => Some(*l),
            _ => None,
        }
    }

    /// Whether control can continue into the following instruction.
    fn falls_through(&self) -> bool {
        !matches!(
            self,
            Op::Throw | Op::ReturnVoid | Op::ReturnValue | Op::Jump(_)
        )
    }

    /// Encoded width in bytes, including the opcode.
    fn width(&self) -> usize {
        1 + match self {
            Op::BitAnd => 0,
            Op::BitOr => 0,
            Op::BitXor => 0,
            Op::LShift => 0,
            Op::RShift => 0,
            Op::URShift => 0,
            Op::BitNot => 0,
            Op::TypeOf => 0,
            Op::IsTypeLate => 0,
            Op::AsTypeLate => 0,
            Op::InstanceOf => 0,
            Op::In => 0,
            Op::NextName => 0,
            Op::NextValue => 0,
            Op::Throw => 0,
            Op::Construct(v) => u30_width(*v),
            Op::NewArray(v) => u30_width(*v),
            Op::NewObject(v) => u30_width(*v),
            Op::ApplyType(v) => u30_width(*v),
            Op::Call(v) => u30_width(*v),
            Op::HasNext2(a, b) => u30_width(*a) + u30_width(*b),
            Op::Nop
            | Op::Label
            | Op::Pop
            | Op::Dup
            | Op::GetLocal0
            | Op::GetLocal1
            | Op::GetLocal2
            | Op::GetLocal3
            | Op::PushNull
            | Op::PushTrue
            | Op::PushFalse
            | Op::PushScope
            | Op::PopScope
            | Op::GetGlobalScope
            | Op::ReturnVoid
            | Op::ReturnValue
            | Op::CoerceAny
            | Op::Negate
            | Op::Not
            | Op::Add
            | Op::Subtract
            | Op::Multiply
            | Op::Divide
            | Op::Modulo
            | Op::Equals
            | Op::StrictEquals
            | Op::LessThan
            | Op::LessEquals
            | Op::GreaterThan
            | Op::GreaterEquals => 0,
            Op::PushByte(_) | Op::GetScopeObject(_) => 1,
            // Branch operands are a fixed-width s24, which is what makes a
            // single sizing pass enough.
            Op::Jump(_)
            | Op::IfTrue(_)
            | Op::IfFalse(_)
            | Op::IfEq(_)
            | Op::IfNe(_)
            | Op::IfStrictEq(_)
            | Op::IfStrictNe(_)
            | Op::IfLt(_)
            | Op::IfLe(_)
            | Op::IfGt(_)
            | Op::IfGe(_) => 3,
            Op::GetLocal(v)
            | Op::SetLocal(v)
            | Op::PushInt(v)
            | Op::PushUInt(v)
            | Op::PushDouble(v)
            | Op::PushString(v)
            | Op::GetLex(v)
            | Op::FindPropStrict(v)
            | Op::GetProperty(v)
            | Op::SetProperty(v)
            | Op::InitProperty(v)
            | Op::ConstructSuper(v)
            | Op::NewClass(v)
            | Op::Coerce(v)
            | Op::GetPropertyIndexed(v)
            | Op::SetPropertyIndexed(v) => u30_width(*v),
            Op::CallProperty { name, arg_count } | Op::CallPropVoid { name, arg_count } => {
                u30_width(*name) + u30_width(*arg_count)
            }
        }
    }
}

fn u30_width(mut value: u32) -> usize {
    let mut n = 1;
    while value >= 0x80 {
        value >>= 7;
        n += 1;
    }
    n
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeStats {
    pub max_stack: u32,
    pub local_count: u32,
    pub max_scope_depth: u32,
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Op(Op),
    Label(Label),
}

#[derive(Debug, Default)]
pub struct CodeBuilder {
    items: Vec<Item>,
    /// One slot per label; `true` once the label has been placed.
    placed: Vec<bool>,
    exceptions: Vec<(Label, Label, Label, u32)>,
}

impl CodeBuilder {
    pub fn catch(&mut self, from: Label, to: Label, target: Label, ty: u32) {
        self.exceptions.push((from, to, target, ty));
    }

    pub fn exception_table(&self) -> Vec<[u32; 5]> {
        let (_, offsets) = self.layout();
        self.exceptions
            .iter()
            .map(|(from, to, target, ty)| {
                [
                    offsets[from.0] as u32,
                    offsets[to.0] as u32,
                    offsets[target.0] as u32,
                    *ty,
                    0,
                ]
            })
            .collect()
    }
    pub fn new() -> Self {
        Self::default()
    }

    pub fn emit(&mut self, op: Op) -> &mut Self {
        self.items.push(Item::Op(op));
        self
    }

    pub fn new_label(&mut self) -> Label {
        self.placed.push(false);
        Label(self.placed.len() - 1)
    }

    pub fn place(&mut self, label: Label) -> &mut Self {
        self.placed[label.0] = true;
        self.items.push(Item::Label(label));
        self
    }

    fn ops(&self) -> impl Iterator<Item = &Op> {
        self.items.iter().filter_map(|i| match i {
            Item::Op(op) => Some(op),
            Item::Label(_) => None,
        })
    }

    /// Byte offset of every op (indexed by op ordinal) and of every label.
    fn layout(&self) -> (Vec<usize>, Vec<usize>) {
        let mut op_offsets = Vec::new();
        let mut label_offsets = vec![0usize; self.placed.len()];
        let mut at = 0usize;
        for item in &self.items {
            match item {
                Item::Op(op) => {
                    op_offsets.push(at);
                    at += op.width();
                }
                Item::Label(l) => label_offsets[l.0] = at,
            }
        }
        (op_offsets, label_offsets)
    }

    /// Serialize to bytecode, resolving every branch to a relative `s24`.
    pub fn assemble(&self) -> Result<Vec<u8>, String> {
        for (i, placed) in self.placed.iter().enumerate() {
            if !placed {
                return Err(format!("label {i} was branched to but never placed"));
            }
        }
        let (op_offsets, label_offsets) = self.layout();
        let mut out = Vec::new();
        for (ordinal, op) in self.ops().enumerate() {
            out.push(op.opcode());
            match op {
                Op::Nop
                | Op::Label
                | Op::Pop
                | Op::Dup
                | Op::GetLocal0
                | Op::GetLocal1
                | Op::GetLocal2
                | Op::GetLocal3
                | Op::PushNull
                | Op::PushTrue
                | Op::PushFalse
                | Op::PushScope
                | Op::PopScope
                | Op::GetGlobalScope
                | Op::ReturnVoid
                | Op::ReturnValue
                | Op::CoerceAny
                | Op::Negate
                | Op::Not
                | Op::Add
                | Op::Subtract
                | Op::Multiply
                | Op::Divide
                | Op::Modulo
                | Op::Equals
                | Op::StrictEquals
                | Op::LessThan
                | Op::LessEquals
                | Op::GreaterThan
                | Op::GreaterEquals => {}
                Op::BitAnd => {}
                Op::BitOr => {}
                Op::BitXor => {}
                Op::LShift => {}
                Op::RShift => {}
                Op::URShift => {}
                Op::BitNot => {}
                Op::TypeOf => {}
                Op::IsTypeLate => {}
                Op::AsTypeLate => {}
                Op::InstanceOf => {}
                Op::In => {}
                Op::NextName => {}
                Op::NextValue => {}
                Op::Throw => {}
                Op::Construct(v) => write_u30(&mut out, *v),
                Op::NewArray(v) => write_u30(&mut out, *v),
                Op::NewObject(v) => write_u30(&mut out, *v),
                Op::ApplyType(v) => write_u30(&mut out, *v),
                Op::Call(v) => write_u30(&mut out, *v),
                Op::HasNext2(a, b) => {
                    write_u30(&mut out, *a);
                    write_u30(&mut out, *b);
                }
                Op::PushByte(v) => out.push(*v as u8),
                Op::GetScopeObject(v) => out.push(*v),
                Op::GetLocal(v)
                | Op::SetLocal(v)
                | Op::PushInt(v)
                | Op::PushUInt(v)
                | Op::PushDouble(v)
                | Op::PushString(v)
                | Op::GetLex(v)
                | Op::FindPropStrict(v)
                | Op::GetProperty(v)
                | Op::SetProperty(v)
                | Op::InitProperty(v)
                | Op::ConstructSuper(v)
                | Op::NewClass(v)
                | Op::Coerce(v)
                | Op::GetPropertyIndexed(v)
                | Op::SetPropertyIndexed(v) => write_u30(&mut out, *v),
                Op::CallProperty { name, arg_count } | Op::CallPropVoid { name, arg_count } => {
                    write_u30(&mut out, *name);
                    write_u30(&mut out, *arg_count);
                }
                _ => {
                    let target = op.branch_target().expect("every remaining op branches");
                    // The operand is relative to the byte after it, and a
                    // branch is always 1 opcode byte + 3 operand bytes wide.
                    let from = op_offsets[ordinal] as i64 + 4;
                    let delta = label_offsets[target.0] as i64 - from;
                    if !(-0x80_0000..0x80_0000).contains(&delta) {
                        return Err(format!("branch offset {delta} does not fit in s24"));
                    }
                    let bytes = (delta as i32).to_le_bytes();
                    out.extend_from_slice(&bytes[..3]);
                }
            }
        }
        Ok(out)
    }

    /// Abstract interpretation over the control-flow graph.
    ///
    /// Each instruction is visited once with the (stack, scope) depth it is
    /// reached at; a second path into the same instruction must agree, which is
    /// exactly the AVM2 verifier's rule. `init_scope_depth` is the depth the
    /// method body inherits from its enclosing scope chain, and the returned
    /// `max_scope_depth` is absolute, as the ABC field is.
    pub fn analyze(&self, init_scope_depth: u32, min_locals: u32) -> Result<CodeStats, String> {
        let ops: Vec<&Op> = self.ops().collect();
        if ops.is_empty() {
            return Err("method body has no instructions".into());
        }
        let (_, label_offsets) = self.layout();
        // Byte offset -> op ordinal, so a label resolves to the op it precedes.
        let mut offset_to_ordinal = std::collections::HashMap::new();
        {
            let mut at = 0usize;
            for (ordinal, op) in ops.iter().enumerate() {
                offset_to_ordinal.insert(at, ordinal);
                at += op.width();
            }
            // A label at the very end targets one past the last instruction.
            offset_to_ordinal.insert(at, ops.len());
        }

        let mut local_count = min_locals.max(1);
        for op in &ops {
            if let Some(r) = op.register() {
                local_count = local_count.max(r + 1);
            }
        }

        let mut seen: Vec<Option<(i32, i32)>> = vec![None; ops.len()];
        let mut max_stack = 0i32;
        let mut max_scope = init_scope_depth as i32;
        let mut work = vec![(0usize, 0i32, init_scope_depth as i32)];
        for (_, _, target, _) in &self.exceptions {
            let ordinal = *offset_to_ordinal
                .get(&label_offsets[target.0])
                .ok_or("catch target is not an instruction")?;
            work.push((ordinal, 1, init_scope_depth as i32));
            max_stack = 1;
        }

        while let Some((mut at, mut stack, mut scope)) = work.pop() {
            loop {
                if at == ops.len() {
                    // Fell off the end of the method. AVM2 requires an explicit
                    // return, so this is a compiler bug rather than a warning.
                    return Err("control reaches the end of a method body without a return".into());
                }
                match seen[at] {
                    Some((s, sc)) => {
                        if s != stack || sc != scope {
                            return Err(format!(
                                "inconsistent depth at instruction {at}: \
                                 reached with stack {stack}/scope {scope}, \
                                 previously stack {s}/scope {sc}"
                            ));
                        }
                        break;
                    }
                    None => seen[at] = Some((stack, scope)),
                }

                let op = ops[at];
                stack += op.stack_delta();
                scope += op.scope_delta();
                if stack < 0 {
                    return Err(format!("operand stack underflows at instruction {at}"));
                }
                if scope < init_scope_depth as i32 {
                    return Err(format!(
                        "scope stack drops below its initial depth at instruction {at}"
                    ));
                }
                max_stack = max_stack.max(stack);
                max_scope = max_scope.max(scope);

                if let Some(target) = op.branch_target() {
                    let ordinal = *offset_to_ordinal
                        .get(&label_offsets[target.0])
                        .ok_or("branch target does not land on an instruction boundary")?;
                    work.push((ordinal, stack, scope));
                }
                if !op.falls_through() {
                    break;
                }
                at += 1;
            }
        }

        Ok(CodeStats {
            max_stack: max_stack as u32,
            local_count,
            max_scope_depth: max_scope as u32,
        })
    }
}
