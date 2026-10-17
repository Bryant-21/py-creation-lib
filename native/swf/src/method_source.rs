//! Independent, conservative AVM2 method recovery and method-only compilation.
//! Unsupported control flow/namespace semantics are errors, never fabricated AS3.
use crate::abc_edit::{decode, edit_movie, merge_class, movie_abcs, name, read_abc};
use crate::abc_inspect::{class_methods, find, string, type_name};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use swf::avm2::types::*;

type Result<T> = std::result::Result<T, String>;

fn selected(abc: &AbcFile, class: usize, method: &str) -> Result<Index<Method>> {
    if method == "constructor" || method.starts_with('$') || method.starts_with("static ") {
        return Err(
            "Constructor, class initializer and static source editing are not supported".into(),
        );
    }
    let matches: Vec<_> = class_methods(abc, class)
        .into_iter()
        .filter(|(label, _)| label == method)
        .collect();
    if matches.len() != 1 {
        return Err(format!("Choose exactly one method/accessor: {method}"));
    }
    Ok(matches[0].1)
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn qualified(value: &str) -> Result<String> {
    if value == "*" || value.split('.').all(identifier) {
        Ok(value.into())
    } else {
        Err(format!(
            "Identifier/type {value:?} needs unsupported namespace handling"
        ))
    }
}

fn signature(abc: &AbcFile, id: Index<Method>) -> Result<(Vec<String>, String)> {
    let info = &abc.methods[id.0 as usize];
    if info
        .flags
        .intersects(MethodFlags::NEED_REST | MethodFlags::NEED_ARGUMENTS | MethodFlags::NATIVE)
    {
        return Err(
            "Rest/arguments/native method signatures cannot be recompiled by this editor".into(),
        );
    }
    let params = info
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            qualified(&type_name(&abc.constant_pool, p.kind))
                .map(|kind| format!("arg{}:{kind}", i + 1))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        params,
        qualified(&type_name(&abc.constant_pool, info.return_type))?,
    ))
}

#[derive(Clone)]
struct Expr {
    value: String,
    start: usize,
    scope: bool,
}

impl Expr {
    fn new(value: String, start: usize) -> Self {
        Self {
            value,
            start,
            scope: false,
        }
    }
}

struct Recovery<'a> {
    abc: &'a AbcFile,
    class: usize,
    ops: Vec<(usize, Op)>,
    end: usize,
    params: usize,
    locals: BTreeSet<u32>,
    serial: usize,
}

impl Recovery<'_> {
    fn pop(stack: &mut Vec<Expr>) -> Result<Expr> {
        stack
            .pop()
            .ok_or_else(|| "Invalid method operand stack".into())
    }
    fn statement_stack(stack: &[Expr]) -> Result<()> {
        if stack.is_empty() {
            Ok(())
        } else {
            Err("Interleaved stack expressions need bytecode inspection to preserve evaluation order".into())
        }
    }
    fn mn(&self, index: Index<Multiname>) -> Result<String> {
        if index.0 == 0 {
            return Err("Wildcard names need bytecode inspection".into());
        }
        let pool = &self.abc.constant_pool;
        match pool.multinames.get(index.0.saturating_sub(1) as usize) {
            Some(Multiname::QName { namespace, .. }) => {
                if !self.namespace(*namespace) {
                    return Err("Custom/foreign namespaces need bytecode inspection".into());
                }
                qualified(&name(pool, index))
            }
            Some(Multiname::Multiname { namespace_set, .. }) => {
                let set = pool
                    .namespace_sets
                    .get(namespace_set.0.saturating_sub(1) as usize)
                    .ok_or("Invalid namespace set")?;
                if set.is_empty() || !set.iter().all(|ns| self.namespace(*ns))
                    || set.iter().any(|ns| matches!(pool.namespaces.get(ns.0.saturating_sub(1) as usize),
                                                  Some(Namespace::Package(value)) if !string(pool, *value).is_empty())) {
                    return Err("Custom/foreign namespace sets need bytecode inspection".into());
                }
                qualified(&name(pool, index))
            }
            _ => Err(
                "Runtime-qualified/attribute/generic multinames are not recoverable as source"
                    .into(),
            ),
        }
    }
    fn namespace(&self, index: Index<Namespace>) -> bool {
        if index.0 == 0 {
            return false;
        }
        let pool = &self.abc.constant_pool;
        match pool.namespaces.get(index.0.saturating_sub(1) as usize) {
            Some(Namespace::Package(_)) => true,
            Some(Namespace::PackageInternal(value)) => {
                let class = name(pool, self.abc.instances[self.class].name);
                string(pool, *value) == class.rsplit_once('.').map_or("", |(package, _)| package)
            }
            Some(Namespace::Private(_))
            | Some(Namespace::Protected(_))
            | Some(Namespace::StaticProtected(_)) => {
                self.abc.instances[self.class].traits.iter().any(|t| {
                    matches!(pool.multinames.get(t.name.0.saturating_sub(1) as usize),
                             Some(Multiname::QName { namespace, .. }) if *namespace == index)
                })
            }
            _ => false,
        }
    }
    fn local(&mut self, index: u32) -> String {
        if index == 0 {
            "this".into()
        } else if index as usize <= self.params {
            format!("arg{index}")
        } else {
            self.locals.insert(index);
            format!("local{index}")
        }
    }
    fn member(&self, owner: Expr, index: Index<Multiname>) -> Result<Expr> {
        let property = self.mn(index)?;
        if !owner.scope {
            if !identifier(&property) {
                return Err("Qualified member access needs bytecode inspection".into());
            }
            let pool = &self.abc.constant_pool;
            if let Some(Multiname::QName { namespace, .. }) =
                pool.multinames.get(index.0.saturating_sub(1) as usize)
            {
                if !matches!(
                    pool.namespaces.get(namespace.0.saturating_sub(1) as usize),
                    Some(Namespace::Package(_))
                ) && owner.value != "this"
                {
                    return Err(
                        "Nonpublic access through another receiver needs bytecode inspection"
                            .into(),
                    );
                }
            }
        }
        Ok(Expr::new(
            if owner.scope {
                property
            } else {
                format!("{}.{}", owner.value, property)
            },
            owner.start,
        ))
    }
    fn boundary(&self, offset: i64) -> Result<usize> {
        if offset == self.end as i64 {
            return Ok(self.ops.len());
        }
        self.ops
            .iter()
            .position(|(at, _)| *at as i64 == offset)
            .ok_or_else(|| "Branch target is not an instruction boundary".into())
    }
    fn range(
        &mut self,
        begin: usize,
        end: usize,
        stack: &mut Vec<Expr>,
        depth: usize,
    ) -> Result<Vec<String>> {
        if depth > 32 {
            return Err("Method control flow exceeds 32 nested blocks".into());
        }
        let mut lines = Vec::new();
        let mut i = begin;
        while i < end {
            let (at, op) = self.ops[i].clone();
            let next = self.ops.get(i + 1).map_or(self.end, |p| p.0);
            let pool = &self.abc.constant_pool;
            let literal =
                match &op {
                    Op::PushByte { value } => Some(value.to_string()),
                    Op::PushShort { value } => Some(value.to_string()),
                    Op::PushInt { value } => {
                        Some(
                            pool.ints
                                .get(value.0.checked_sub(1).ok_or(
                                    "Implicit integer pool constants need bytecode inspection",
                                )? as usize)
                                .ok_or("Bad integer index")?
                                .to_string(),
                        )
                    }
                    Op::PushUint { value } => {
                        Some(format!(
                            "uint({})",
                            pool.uints
                                .get(value.0.checked_sub(1).ok_or(
                                    "Implicit uint pool constants need bytecode inspection"
                                )? as usize)
                                .ok_or("Bad uint index")?
                        ))
                    }
                    Op::PushDouble { value } => {
                        let number =
                            *pool
                                .doubles
                                .get(value.0.checked_sub(1).ok_or(
                                    "Implicit double pool constants need bytecode inspection",
                                )? as usize)
                                .ok_or("Bad double index")?;
                        if !number.is_finite() {
                            return Err(
                                "Non-finite numeric constants need bytecode inspection".into()
                            );
                        }
                        Some(format!("Number({number:?})"))
                    }
                    Op::PushString { value } => {
                        Some(serde_json::to_string(&string(pool, *value)).unwrap())
                    }
                    Op::PushTrue => Some("true".into()),
                    Op::PushFalse => Some("false".into()),
                    Op::PushNull => Some("null".into()),
                    Op::PushUndefined => Some("undefined".into()),
                    _ => None,
                };
            if let Some(value) = literal {
                stack.push(Expr::new(value, at));
                i += 1;
                continue;
            }
            match op {
                Op::Nop
                | Op::Label
                | Op::Debug { .. }
                | Op::DebugFile { .. }
                | Op::DebugLine { .. } => {}
                Op::GetLocal { index } => {
                    let value = self.local(index);
                    stack.push(Expr::new(value, at));
                }
                Op::SetLocal { index } => {
                    let value = Self::pop(stack)?;
                    Self::statement_stack(stack)?;
                    if index == 0 {
                        return Err("Assigning receiver local needs bytecode inspection".into());
                    }
                    let local = self.local(index);
                    lines.push(format!("{local} = {};", value.value));
                }
                Op::Kill { index } => {
                    Self::statement_stack(stack)?;
                    if index == 0 {
                        return Err("Killing receiver local needs bytecode inspection".into());
                    }
                    let local = self.local(index);
                    lines.push(format!("{local} = undefined;"));
                }
                Op::PushScope => {
                    let scope = Self::pop(stack)?;
                    if at > 1 || scope.value != "this" {
                        return Err("Dynamic scope changes need bytecode inspection".into());
                    }
                }
                Op::FindPropStrict { index } => stack.push(Expr {
                    value: self.mn(index)?,
                    start: at,
                    scope: true,
                }),
                Op::GetLex { index } => stack.push(Expr::new(self.mn(index)?, at)),
                Op::GetProperty { index } => {
                    let owner = Self::pop(stack)?;
                    stack.push(self.member(owner, index)?);
                }
                Op::SetProperty { index } => {
                    let value = Self::pop(stack)?;
                    let owner = Self::pop(stack)?;
                    Self::statement_stack(stack)?;
                    let member = self.member(owner, index)?;
                    lines.push(format!("{} = {};", member.value, value.value));
                }
                Op::CallProperty { index, num_args }
                | Op::CallPropVoid { index, num_args }
                | Op::ConstructProp { index, num_args } => {
                    let mut args = Vec::new();
                    for _ in 0..num_args {
                        args.push(Self::pop(stack)?.value);
                    }
                    args.reverse();
                    let owner = Self::pop(stack)?;
                    let function = self.member(owner, index)?;
                    let value = format!(
                        "{}{}({})",
                        if matches!(op, Op::ConstructProp { .. }) {
                            "new "
                        } else {
                            ""
                        },
                        function.value,
                        args.join(", ")
                    );
                    if matches!(op, Op::CallPropVoid { .. }) {
                        Self::statement_stack(stack)?;
                        lines.push(value + ";");
                    } else {
                        stack.push(Expr::new(value, function.start));
                    }
                }
                Op::ReturnValue => {
                    let value = Self::pop(stack)?;
                    lines.push(format!("return {};", value.value));
                }
                Op::ReturnVoid => lines.push("return;".into()),
                Op::Throw => {
                    let value = Self::pop(stack)?;
                    lines.push(format!("throw {};", value.value));
                }
                Op::Pop => {
                    let value = Self::pop(stack)?;
                    Self::statement_stack(stack)?;
                    lines.push(value.value + ";");
                }
                Op::Dup => {
                    let value = Self::pop(stack)?;
                    Self::statement_stack(stack)?;
                    self.serial += 1;
                    let local = format!("value{}", self.serial);
                    lines.push(format!("var {local}:* = {};", value.value));
                    stack.push(Expr::new(local.clone(), value.start));
                    stack.push(Expr::new(local, value.start));
                }
                Op::CoerceA => {}
                Op::ConvertB
                | Op::ConvertD
                | Op::ConvertI
                | Op::ConvertU
                | Op::ConvertS
                | Op::Not
                | Op::Negate
                | Op::TypeOf => {
                    let value = Self::pop(stack)?;
                    let converted = match op {
                        Op::ConvertB => format!("Boolean({})", value.value),
                        Op::ConvertD => format!("Number({})", value.value),
                        Op::ConvertI => format!("int({})", value.value),
                        Op::ConvertU => format!("uint({})", value.value),
                        Op::ConvertS => format!("String({})", value.value),
                        Op::Not => format!("!({})", value.value),
                        Op::Negate => format!("-({})", value.value),
                        _ => format!("typeof ({})", value.value),
                    };
                    stack.push(Expr::new(converted, value.start));
                }
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
                | Op::GreaterEquals
                | Op::BitAnd
                | Op::BitOr
                | Op::BitXor
                | Op::LShift
                | Op::RShift
                | Op::URShift => {
                    let right = Self::pop(stack)?;
                    let left = Self::pop(stack)?;
                    let symbol = match op {
                        Op::Add => "+",
                        Op::Subtract => "-",
                        Op::Multiply => "*",
                        Op::Divide => "/",
                        Op::Modulo => "%",
                        Op::Equals => "==",
                        Op::StrictEquals => "===",
                        Op::LessThan => "<",
                        Op::LessEquals => "<=",
                        Op::GreaterThan => ">",
                        Op::GreaterEquals => ">=",
                        Op::BitAnd => "&",
                        Op::BitOr => "|",
                        Op::BitXor => "^",
                        Op::LShift => "<<",
                        Op::RShift => ">>",
                        _ => ">>>",
                    };
                    stack.push(Expr::new(
                        format!("({} {symbol} {})", left.value, right.value),
                        left.start.min(right.start),
                    ));
                }
                Op::IfFalse { offset } | Op::IfTrue { offset } => {
                    let condition = Self::pop(stack)?;
                    if !stack.is_empty() {
                        return Err(
                            "Branch joins with live operands are not recoverable as source".into(),
                        );
                    }
                    let target = self.boundary(next as i64 + offset as i64)?;
                    if target <= i || target > end {
                        return Err(
                            "Unstructured/backward branch is not recoverable as source".into()
                        );
                    }
                    let condition_value = if matches!(op, Op::IfTrue { .. }) {
                        format!("!({})", condition.value)
                    } else {
                        condition.value
                    };
                    let mut true_end = target;
                    let mut after = target;
                    let mut otherwise = None;
                    let mut keyword = "if";
                    if target > i + 1 {
                        if let Op::Jump { offset } = self.ops[target - 1].1 {
                            let branch_next = self.ops.get(target).map_or(self.end, |p| p.0);
                            let jump = self.boundary(branch_next as i64 + offset as i64)?;
                            if jump <= i {
                                if self.ops[jump].0 != condition.start {
                                    return Err(
                                        "Loop condition has unsupported side effects/control flow"
                                            .into(),
                                    );
                                }
                                keyword = "while";
                                true_end -= 1;
                            } else if jump > target && jump <= end {
                                true_end -= 1;
                                after = jump;
                                otherwise = Some((target, jump));
                            } else {
                                return Err("Unstructured conditional jump".into());
                            }
                        }
                    }
                    let body = self.range(i + 1, true_end, &mut Vec::new(), depth + 1)?;
                    lines.push(format!("{keyword} ({condition_value}) {{"));
                    lines.extend(body.into_iter().map(|line| "    ".to_string() + &line));
                    lines.push("}".into());
                    if let Some((begin, end)) = otherwise {
                        let body = self.range(begin, end, &mut Vec::new(), depth + 1)?;
                        lines.push("else {".into());
                        lines.extend(body.into_iter().map(|line| "    ".to_string() + &line));
                        lines.push("}".into());
                    }
                    i = after;
                    continue;
                }
                _ => return Err(format!("Unsupported source recovery at byte {at}: {op:?}")),
            }
            i += 1;
        }
        if !stack.is_empty() {
            return Err("Method/block leaves operands on the stack".into());
        }
        Ok(lines)
    }
}

pub fn recover(data: &[u8], class: &str, method: &str) -> Result<Value> {
    let abcs = movie_abcs(data)?;
    let found = find(&abcs, class)?;
    let abc = found.abc;
    let id = selected(abc, found.index, method)?;
    let (params, returns) = signature(abc, id)?;
    let body = abc
        .method_bodies
        .iter()
        .find(|b| b.method == id)
        .ok_or("Method has no bytecode body")?;
    if !body.exceptions.is_empty() || !body.traits.is_empty() {
        return Err("Exception handlers/activation traits need bytecode inspection; source recovery is unsupported".into());
    }
    let mut recovery = Recovery {
        abc,
        class: found.index,
        ops: decode(&body.code)?,
        end: body.code.len(),
        params: params.len(),
        locals: BTreeSet::new(),
        serial: 0,
    };
    let count = recovery.ops.len();
    let lines = recovery.range(0, count, &mut Vec::new(), 0)?;
    let generated: BTreeSet<String> = (1..=params.len())
        .map(|i| format!("arg{i}"))
        .chain(recovery.locals.iter().map(|i| format!("local{i}")))
        .chain((1..=recovery.serial).map(|i| format!("value{i}")))
        .collect();
    if abc
        .constant_pool
        .multinames
        .iter()
        .enumerate()
        .any(|(i, _)| generated.contains(&name(&abc.constant_pool, Index::new(i as u32 + 1))))
    {
        return Err("Recovered local/parameter names conflict with existing member names; use bytecode inspection".into());
    }
    let mut source = String::new();
    for index in &recovery.locals {
        source.push_str(&format!("var local{index}:*;\n"));
    }
    source.push_str(&lines.join("\n"));
    Ok(
        json!({"class":class,"method":method,"parameters":params,"returns":returns,"body":source,
              "coverage":"Conservative native method recovery; unsupported AVM2 patterns are rejected"}),
    )
}

pub fn replace(data: &[u8], class: &str, method: &str, source: &str) -> Result<Vec<u8>> {
    let mut types = HashMap::new();
    for abc in movie_abcs(data)? {
        for instance in &abc.instances {
            types.insert(
                name(&abc.constant_pool, instance.name),
                name(&abc.constant_pool, instance.super_name),
            );
        }
    }
    let mut count = 0;
    let result = edit_movie(data, |abc| {
        let Some(class_index) = abc
            .instances
            .iter()
            .position(|i| name(&abc.constant_pool, i.name) == class)
        else {
            return Ok(false);
        };
        let method_id = selected(abc, class_index, method)?;
        let (params, returns) = signature(abc, method_id)?;
        let target_body = abc
            .method_bodies
            .iter()
            .position(|b| b.method == method_id)
            .ok_or("Method has no body")?;
        let original_scope = abc.method_bodies[target_body].init_scope_depth;
        let (package, simple) = class.rsplit_once('.').unwrap_or(("", class));
        qualified(class)?;
        let superclass = qualified(&type_name(
            &abc.constant_pool,
            abc.instances[class_index].super_name,
        ))?;
        let temporary = format!("B21_MethodEdit_{}", abc.methods.len());
        if abc.instances[class_index]
            .traits
            .iter()
            .any(|t| name(&abc.constant_pool, t.name) == temporary)
        {
            return Err("Temporary method name collision".into());
        }
        let wrapper = format!(
            "package {package} {{ public class {simple} extends {superclass} {{ public function {temporary}({}):{returns} {{\n{source}\n}} }} }}",
            params.join(", ")
        );
        let compiled = as3_native::compile_sources_with_types(&[&wrapper], &types)
            .map_err(|e| format!("{class}.{method}: {e}"))?;
        let compiled = read_abc(&compiled)?;
        validate_private_receivers(abc, class_index, &compiled, &temporary)?;
        merge_class(abc, compiled, class_index, true)?;
        let temporary_trait = abc.instances[class_index]
            .traits
            .iter()
            .position(|t| name(&abc.constant_pool, t.name) == temporary)
            .ok_or("Compiler did not emit the edited method")?;
        let temporary_id = match abc.instances[class_index]
            .traits
            .remove(temporary_trait)
            .kind
        {
            TraitKind::Method { method, .. } => method,
            _ => return Err("Edited method has unexpected trait kind".into()),
        };
        let new_body = abc
            .method_bodies
            .iter()
            .position(|b| b.method == temporary_id)
            .ok_or("Compiler did not emit a method body")?;
        let mut replacement = abc.method_bodies.remove(new_body);
        let scope_delta = original_scope as i64 - replacement.init_scope_depth as i64;
        replacement.max_scope_depth = (replacement.max_scope_depth as i64 + scope_delta)
            .try_into()
            .map_err(|_| "Invalid method scope depth")?;
        replacement.init_scope_depth = original_scope;
        replacement.method = method_id;
        let runtime =
            MethodFlags::NEED_ARGUMENTS | MethodFlags::NEED_ACTIVATION | MethodFlags::SET_DXNS;
        let flags = abc.methods[temporary_id.0 as usize].flags;
        abc.methods[method_id.0 as usize].flags.remove(runtime);
        abc.methods[method_id.0 as usize]
            .flags
            .insert(flags & runtime);
        // The old method ID/signature, trait namespace, accessor kind and all
        // other original method bytecode remain unchanged. Only this body moves.
        let target_body = abc
            .method_bodies
            .iter()
            .position(|b| b.method == method_id)
            .ok_or("Original body vanished")?;
        abc.method_bodies[target_body] = replacement;
        count += 1;
        Ok(true)
    })?;
    if count != 1 {
        return Err("Expected exactly one target class/method".into());
    }
    Ok(result)
}

// merge_class restores original nonpublic trait namespaces by member name.
// Prove those accesses use this/the lexical receiver before doing that remap;
// another object's public member with the same name must never become private.
fn validate_private_receivers(
    original: &AbcFile,
    class: usize,
    compiled: &AbcFile,
    temporary: &str,
) -> Result<()> {
    let pool = &original.constant_pool;
    let mut members = HashMap::new();
    let mut nonpublic = BTreeSet::new();
    for t in original.instances[class]
        .traits
        .iter()
        .chain(original.classes[class].traits.iter())
    {
        let multiname = &pool.multinames[t.name.0 as usize - 1];
        let key = name(pool, t.name);
        if members
            .insert(key.clone(), multiname)
            .is_some_and(|old| old != multiname)
        {
            return Err("Ambiguous member namespaces cannot be recompiled as method source".into());
        }
        if let Multiname::QName { namespace, .. } = multiname {
            if !matches!(
                pool.namespaces.get(namespace.0.saturating_sub(1) as usize),
                Some(Namespace::Package(_))
            ) {
                nonpublic.insert(key);
            }
        }
    }
    if nonpublic.is_empty() {
        return Ok(());
    }
    let id = class_methods(compiled, 0)
        .into_iter()
        .find(|(label, _)| label == temporary)
        .ok_or("Compiled method is unavailable")?
        .1;
    let body = compiled
        .method_bodies
        .iter()
        .find(|b| b.method == id)
        .ok_or("Compiled method has no body")?;
    let mut stack = Vec::new();
    for (_, op) in decode(&body.code)? {
        match op {
            Op::GetLocal { index } => stack.push(index == 0),
            Op::FindProperty { .. } | Op::FindPropStrict { .. } => stack.push(true),
            Op::PushByte { .. }
            | Op::PushShort { .. }
            | Op::PushInt { .. }
            | Op::PushUint { .. }
            | Op::PushDouble { .. }
            | Op::PushString { .. }
            | Op::PushTrue
            | Op::PushFalse
            | Op::PushNull
            | Op::PushUndefined
            | Op::GetLex { .. } => stack.push(false),
            Op::PushScope | Op::SetLocal { .. } | Op::Pop => {
                stack.pop();
            }
            Op::Dup => {
                let v = stack.last().copied().unwrap_or(false);
                stack.push(v);
            }
            Op::CoerceA
            | Op::Nop
            | Op::Label
            | Op::Debug { .. }
            | Op::DebugFile { .. }
            | Op::DebugLine { .. } => {}
            Op::GetProperty { index }
            | Op::SetProperty { index }
            | Op::CallProperty { index, .. }
            | Op::CallPropVoid { index, .. }
            | Op::ConstructProp { index, .. } => {
                let args = match op {
                    Op::SetProperty { .. } => 1,
                    Op::CallProperty { num_args, .. }
                    | Op::CallPropVoid { num_args, .. }
                    | Op::ConstructProp { num_args, .. } => num_args,
                    _ => 0,
                };
                for _ in 0..args {
                    stack.pop();
                }
                let receiver = stack.pop().unwrap_or(false);
                if nonpublic.contains(&name(&compiled.constant_pool, index)) && !receiver {
                    return Err(
                        "Cannot prove nonpublic member receiver; use bytecode inspection instead"
                            .into(),
                    );
                }
                if !matches!(op, Op::SetProperty { .. } | Op::CallPropVoid { .. }) {
                    stack.push(false);
                }
            }
            _ => stack.clear(), // Unknown stack effects/control flow cannot prove a receiver.
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abc() -> AbcFile {
        let bytes = as3_native::compile_sources_with_types(
            &["package { public class B21_Recovery { public function value(arg1:int):int { return arg1; } } }"],
            &HashMap::new(),
        ).unwrap();
        read_abc(&bytes).unwrap()
    }

    #[test]
    fn reject_interleaved_local_mutation_instead_of_delaying_its_read() {
        let abc = abc();
        let mut recovery = Recovery {
            abc: &abc,
            class: 0,
            params: 1,
            locals: BTreeSet::new(),
            serial: 0,
            ops: vec![
                (0, Op::GetLocal { index: 1 }),
                (1, Op::PushByte { value: 3 }),
                (3, Op::SetLocal { index: 1 }),
                (4, Op::ReturnValue),
            ],
            end: 5,
        };
        assert!(
            recovery
                .range(0, 4, &mut Vec::new(), 0)
                .unwrap_err()
                .contains("evaluation order")
        );
    }

    #[test]
    fn reject_custom_namespace_instead_of_dropping_its_uri() {
        let mut abc = abc();
        let trait_name = abc.instances[0].traits[0].name;
        let Multiname::QName { namespace, .. } =
            abc.constant_pool.multinames[trait_name.0 as usize - 1]
        else {
            panic!()
        };
        abc.constant_pool.namespaces[namespace.0 as usize - 1] = Namespace::Explicit(Index::new(0));
        let recovery = Recovery {
            abc: &abc,
            class: 0,
            params: 1,
            locals: BTreeSet::new(),
            serial: 0,
            ops: vec![],
            end: 0,
        };
        assert!(recovery.mn(trait_name).unwrap_err().contains("namespace"));
    }
}
