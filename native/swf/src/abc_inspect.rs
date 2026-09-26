//! Read-only views of AVM2 classes for writing menu bridges without JPEXS:
//! outline, disassembly and the names a class depends on.

use crate::abc_edit::{decode, movie_abcs, name};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet, VecDeque};
use swf::avm2::types::*;

type Result<T> = std::result::Result<T, String>;

pub(crate) struct Found<'a> {
    pub abc: &'a AbcFile,
    pub index: usize,
}

pub(crate) fn find<'a>(abcs: &'a [AbcFile], class: &str) -> Result<Found<'a>> {
    abcs.iter()
        .find_map(|abc| {
            abc.instances
                .iter()
                .position(|i| name(&abc.constant_pool, i.name) == class)
                .map(|index| Found { abc, index })
        })
        .ok_or_else(|| format!("Class '{class}' is not defined in this movie"))
}

// `abc_edit::name` renders parameterised types as "*"; outlines need Vector.<T>.
pub(crate) fn type_name(pool: &ConstantPool, index: Index<Multiname>) -> String {
    if index.0 == 0 {
        return "*".into();
    }
    match &pool.multinames[index.0 as usize - 1] {
        Multiname::TypeName { base_type, parameters } => {
            let parameters: Vec<_> = parameters.iter().map(|p| type_name(pool, *p)).collect();
            format!("{}.<{}>", type_name(pool, *base_type), parameters.join(", "))
        }
        _ => name(pool, index),
    }
}

pub(crate) fn string(pool: &ConstantPool, index: Index<String>) -> String {
    match index.0.checked_sub(1).and_then(|i| pool.strings.get(i as usize)) {
        Some(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        None => String::new(),
    }
}

pub(crate) fn signature(abc: &AbcFile, method: Index<Method>) -> Value {
    let pool = &abc.constant_pool;
    let info = &abc.methods[method.0 as usize];
    json!({
        "params": info.params.iter().map(|p| type_name(pool, p.kind)).collect::<Vec<_>>(),
        "optional": info.params.iter().filter(|p| p.default_value.is_some()).count(),
        "rest": info.flags.contains(MethodFlags::NEED_REST),
        "returns": type_name(pool, info.return_type),
    })
}

fn trait_outline(abc: &AbcFile, t: &Trait) -> Value {
    let pool = &abc.constant_pool;
    let trait_name = name(pool, t.name);
    match &t.kind {
        TraitKind::Slot { type_name: ty, .. } => {
            json!({"name": trait_name, "kind": "var", "type": type_name(pool, *ty)})
        }
        TraitKind::Const { type_name: ty, .. } => {
            json!({"name": trait_name, "kind": "const", "type": type_name(pool, *ty)})
        }
        TraitKind::Method { method, .. } => {
            json!({"name": trait_name, "kind": "method", "signature": signature(abc, *method)})
        }
        TraitKind::Getter { method, .. } => {
            json!({"name": trait_name, "kind": "getter", "signature": signature(abc, *method)})
        }
        TraitKind::Setter { method, .. } => {
            json!({"name": trait_name, "kind": "setter", "signature": signature(abc, *method)})
        }
        TraitKind::Function { function, .. } => {
            json!({"name": trait_name, "kind": "function", "signature": signature(abc, *function)})
        }
        TraitKind::Class { class, .. } => {
            json!({"name": trait_name, "kind": "class", "type": name(pool, abc.instances[class.0 as usize].name)})
        }
    }
}

/// Every callable of a class with the label users select it by: `constructor`,
/// `static initializer`, `name`, `get name`, `set name`, each optionally `static `-prefixed.
pub(crate) fn class_methods(abc: &AbcFile, index: usize) -> Vec<(String, Index<Method>)> {
    let pool = &abc.constant_pool;
    let mut methods = vec![
        ("constructor".to_string(), abc.instances[index].init_method),
        ("static initializer".to_string(), abc.classes[index].init_method),
    ];
    let instance = abc.instances[index].traits.iter().map(|t| (t, ""));
    let statics = abc.classes[index].traits.iter().map(|t| (t, "static "));
    for (t, prefix) in instance.chain(statics) {
        let trait_name = name(pool, t.name);
        match &t.kind {
            TraitKind::Method { method, .. } => methods.push((format!("{prefix}{trait_name}"), *method)),
            TraitKind::Getter { method, .. } => methods.push((format!("{prefix}get {trait_name}"), *method)),
            TraitKind::Setter { method, .. } => methods.push((format!("{prefix}set {trait_name}"), *method)),
            TraitKind::Function { function, .. } => {
                methods.push((format!("{prefix}{trait_name}"), *function))
            }
            _ => {}
        }
    }
    methods
}

pub fn class_outline(data: &[u8], class: &str) -> Result<Value> {
    let abcs = movie_abcs(data)?;
    let Found { abc, index } = find(&abcs, class)?;
    let pool = &abc.constant_pool;
    let instance = &abc.instances[index];
    let mut flags = Vec::new();
    if instance.is_sealed {
        flags.push("sealed");
    }
    if instance.is_final {
        flags.push("final");
    }
    if instance.is_interface {
        flags.push("interface");
    }
    Ok(json!({
        "name": class,
        "super": type_name(pool, instance.super_name),
        "interfaces": instance.interfaces.iter().map(|i| type_name(pool, *i)).collect::<Vec<_>>(),
        "flags": flags,
        "constructor": signature(abc, instance.init_method),
        "instance_traits": instance.traits.iter().map(|t| trait_outline(abc, t)).collect::<Vec<_>>(),
        "static_traits": abc.classes[index].traits.iter().map(|t| trait_outline(abc, t)).collect::<Vec<_>>(),
    }))
}

fn pooled<T: ToString>(values: &[T], index: u32) -> String {
    index
        .checked_sub(1)
        .and_then(|i| values.get(i as usize))
        .map_or_else(|| "?".into(), ToString::to_string)
}

fn instruction(abc: &AbcFile, op: &Op, offset: usize, next: usize) -> String {
    let pool = &abc.constant_pool;
    let debug = format!("{op:?}");
    let mnemonic = debug.split([' ', '(']).next().unwrap_or_default();
    let target = |relative: i32| next as i64 + relative as i64;
    match op {
        Op::GetProperty { index }
        | Op::SetProperty { index }
        | Op::InitProperty { index }
        | Op::DeleteProperty { index }
        | Op::GetLex { index }
        | Op::FindProperty { index }
        | Op::FindPropStrict { index }
        | Op::FindDef { index }
        | Op::GetSuper { index }
        | Op::SetSuper { index }
        | Op::GetDescendants { index }
        | Op::Coerce { index }
        | Op::IsType { index }
        | Op::AsType { type_name: index } => format!("{mnemonic} {}", type_name(pool, *index)),
        Op::CallProperty { index, num_args }
        | Op::CallPropLex { index, num_args }
        | Op::CallPropVoid { index, num_args }
        | Op::CallSuper { index, num_args }
        | Op::CallSuperVoid { index, num_args }
        | Op::ConstructProp { index, num_args } => {
            format!("{mnemonic} {} ({num_args})", type_name(pool, *index))
        }
        Op::PushString { value } => format!("{mnemonic} {:?}", string(pool, *value)),
        Op::PushInt { value } => format!("{mnemonic} {}", pooled(&pool.ints, value.0)),
        Op::PushUint { value } => format!("{mnemonic} {}", pooled(&pool.uints, value.0)),
        Op::PushDouble { value } => format!("{mnemonic} {}", pooled(&pool.doubles, value.0)),
        Op::DebugFile { file_name } => format!("{mnemonic} {:?}", string(pool, *file_name)),
        Op::NewClass { index } => format!(
            "{mnemonic} {}",
            abc.instances.get(index.0 as usize).map_or_else(|| "?".into(), |i| name(pool, i.name))
        ),
        Op::CallStatic { index, num_args } => format!("{mnemonic} method#{} ({num_args})", index.0),
        Op::NewFunction { index } => format!("{mnemonic} method#{}", index.0),
        Op::Jump { offset: relative }
        | Op::IfTrue { offset: relative }
        | Op::IfFalse { offset: relative }
        | Op::IfEq { offset: relative }
        | Op::IfNe { offset: relative }
        | Op::IfLt { offset: relative }
        | Op::IfLe { offset: relative }
        | Op::IfGt { offset: relative }
        | Op::IfGe { offset: relative }
        | Op::IfNlt { offset: relative }
        | Op::IfNle { offset: relative }
        | Op::IfNgt { offset: relative }
        | Op::IfNge { offset: relative }
        | Op::IfStrictEq { offset: relative }
        | Op::IfStrictNe { offset: relative } => format!("{mnemonic} -> {}", target(*relative)),
        // lookupswitch offsets are relative to the switch instruction itself, not the next one.
        Op::LookupSwitch(table) => {
            let base = offset as i64;
            let cases: Vec<_> = table.case_offsets.iter().map(|c| (base + *c as i64).to_string()).collect();
            format!("{mnemonic} -> {}, cases [{}]", base + table.default_offset as i64, cases.join(", "))
        }
        _ => debug,
    }
}

pub fn disassemble(data: &[u8], class: &str, method: Option<&str>) -> Result<Value> {
    let abcs = movie_abcs(data)?;
    let Found { abc, index } = find(&abcs, class)?;
    let pool = &abc.constant_pool;
    let selected: Vec<_> = class_methods(abc, index)
        .into_iter()
        .filter(|(label, _)| {
            method.is_none_or(|wanted| label == wanted || label.rsplit(' ').next() == Some(wanted))
        })
        .collect();
    if selected.is_empty() {
        return Err(format!("Class '{class}' has no method '{}'", method.unwrap_or_default()));
    }
    let mut listing = Vec::new();
    for (label, id) in selected {
        let Some(body) = abc.method_bodies.iter().find(|b| b.method.0 == id.0) else {
            listing.push(json!({"method": label, "signature": signature(abc, id), "native": true, "code": [], "exceptions": []}));
            continue;
        };
        let ops = decode(&body.code)?;
        let code: Vec<String> = ops
            .iter()
            .enumerate()
            .map(|(i, (offset, op))| {
                let next = ops.get(i + 1).map_or(body.code.len(), |p| p.0);
                format!("{offset:>5}  {}", instruction(abc, op, *offset, next))
            })
            .collect();
        let exceptions: Vec<Value> = body
            .exceptions
            .iter()
            .map(|e| {
                json!({
                    "from": e.from_offset,
                    "to": e.to_offset,
                    "target": e.target_offset,
                    "type": type_name(pool, e.type_name),
                    "variable": type_name(pool, e.variable_name),
                })
            })
            .collect();
        listing.push(json!({"method": label, "signature": signature(abc, id), "native": false, "code": code, "exceptions": exceptions}));
    }
    Ok(Value::Array(listing))
}

const TOP_LEVEL: &[&str] = &[
    "Object", "Array", "String", "Number", "int", "uint", "Boolean", "Function", "Class",
    "Math", "Date", "RegExp", "Error", "TypeError", "RangeError", "ArgumentError",
    "ReferenceError", "SecurityError", "XML", "XMLList", "Namespace", "QName", "JSON",
    "Vector", "__AS3__.vec.Vector", "void", "trace", "isNaN", "isFinite", "parseInt",
    "parseFloat", "escape", "unescape", "encodeURI", "encodeURIComponent", "decodeURI",
    "decodeURIComponent", "undefined", "NaN", "Infinity",
];

fn is_builtin(name: &str) -> bool {
    ["flash.", "scaleform.", "adobe."].iter().any(|p| name.starts_with(p)) || TOP_LEVEL.contains(&name)
}

fn starts_lowercase_unqualified(name: &str) -> bool {
    !name.contains('.') && name.starts_with(|c: char| c.is_ascii_lowercase())
}

fn collect_types(pool: &ConstantPool, index: Index<Multiname>, out: &mut BTreeSet<String>) {
    if index.0 == 0 {
        return;
    }
    match &pool.multinames[index.0 as usize - 1] {
        Multiname::TypeName { base_type, parameters } => {
            collect_types(pool, *base_type, out);
            for parameter in parameters {
                collect_types(pool, *parameter, out);
            }
        }
        _ => {
            out.insert(name(pool, index));
        }
    }
}

fn own_trait_names(abcs: &[AbcFile], class: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut current = class.to_string();
    while let Ok(Found { abc, index }) = find(abcs, &current) {
        let pool = &abc.constant_pool;
        for t in abc.instances[index].traits.iter().chain(&abc.classes[index].traits) {
            names.insert(name(pool, t.name));
        }
        let parent = type_name(pool, abc.instances[index].super_name);
        if parent == current {
            break;
        }
        current = parent;
    }
    names
}

fn class_report(abcs: &[AbcFile], defined_names: &HashSet<String>, class: &str) -> Result<Value> {
    let Found { abc, index } = find(abcs, class)?;
    let pool = &abc.constant_pool;
    let own = own_trait_names(abcs, class);
    let mut lexical = BTreeSet::new();
    let mut members = BTreeSet::new();
    let instance = &abc.instances[index];
    collect_types(pool, instance.super_name, &mut lexical);
    for interface in &instance.interfaces {
        collect_types(pool, *interface, &mut lexical);
    }
    for t in instance.traits.iter().chain(&abc.classes[index].traits) {
        if let TraitKind::Slot { type_name: ty, .. } | TraitKind::Const { type_name: ty, .. } = &t.kind {
            collect_types(pool, *ty, &mut lexical);
        }
    }
    for (_, id) in class_methods(abc, index) {
        let info = &abc.methods[id.0 as usize];
        collect_types(pool, info.return_type, &mut lexical);
        for parameter in &info.params {
            collect_types(pool, parameter.kind, &mut lexical);
        }
        let Some(body) = abc.method_bodies.iter().find(|b| b.method.0 == id.0) else {
            continue;
        };
        for (_, op) in decode(&body.code)? {
            match op {
                Op::GetLex { index }
                | Op::FindDef { index }
                | Op::Coerce { index }
                | Op::IsType { index }
                | Op::AsType { type_name: index }
                | Op::ConstructProp { index, .. } => collect_types(pool, index, &mut lexical),
                // findpropstrict/findproperty precede both `new X` and unqualified member calls;
                // a class name reaches `lexical` through the matching getlex/constructprop instead.
                Op::FindPropStrict { index }
                | Op::FindProperty { index }
                | Op::GetProperty { index }
                | Op::SetProperty { index }
                | Op::InitProperty { index }
                | Op::DeleteProperty { index }
                | Op::CallProperty { index, .. }
                | Op::CallPropVoid { index, .. }
                | Op::CallPropLex { index, .. } => {
                    members.insert(name(pool, index));
                }
                _ => {}
            }
        }
    }
    let (mut defined, mut builtin, mut external) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for candidate in lexical {
        if candidate == "*" || candidate == class {
            continue;
        }
        if defined_names.contains(&candidate) {
            defined.insert(candidate);
        } else if is_builtin(&candidate) {
            builtin.insert(candidate);
        } else if own.contains(&candidate) || starts_lowercase_unqualified(&candidate) {
            // Flash-compiled code reaches inherited display-object properties
            // (stage, parent, mouseX) through getlex; they are members, not classes.
            members.insert(candidate);
        } else {
            external.insert(candidate);
        }
    }
    members.remove("*");
    members.retain(|m| !defined.contains(m) && !is_builtin(m));
    Ok(json!({"class": class, "defined": defined, "builtin": builtin, "external": external, "members": members}))
}

pub fn class_references(data: &[u8], class: &str, transitive: bool) -> Result<Value> {
    let abcs = movie_abcs(data)?;
    find(&abcs, class)?;
    let defined_names: HashSet<String> = abcs
        .iter()
        .flat_map(|abc| abc.instances.iter().map(|i| name(&abc.constant_pool, i.name)))
        .collect();
    let mut reports = Vec::new();
    let mut external = BTreeSet::new();
    let mut seen = HashSet::from([class.to_string()]);
    let mut queue = VecDeque::from([class.to_string()]);
    while let Some(current) = queue.pop_front() {
        let report = class_report(&abcs, &defined_names, &current)?;
        for entry in report["external"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            external.insert(entry.to_string());
        }
        if transitive {
            for next in report["defined"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                if seen.insert(next.to_string()) {
                    queue.push_back(next.to_string());
                }
            }
        }
        reports.push(report);
    }
    Ok(json!({"root": class, "classes": reports, "external": external}))
}
