use crate::abc::abc_block_offset;
use crate::container::{assemble, decompress, split_tags, tags_offset, write_tag_header};
use std::collections::{BTreeMap, HashMap};
use swf::avm2::{read::Reader, types::*, write::Writer};
use swf::extensions::ReadSwfExt;

type Result<T> = std::result::Result<T, String>;

pub fn read_abc(data: &[u8]) -> Result<AbcFile> {
    Reader::new(data).read().map_err(|e| e.to_string())
}

pub fn name(pool: &ConstantPool, index: Index<Multiname>) -> String {
    if index.0 == 0 {
        return "*".into();
    }
    match &pool.multinames[index.0 as usize - 1] {
        Multiname::QName { namespace, name } | Multiname::QNameA { namespace, name } => {
            let ns = match pool.namespaces[namespace.0 as usize - 1] {
                Namespace::Package(s) | Namespace::PackageInternal(s) => string(pool, s),
                _ => String::new(),
            };
            let value = string(pool, *name);
            if ns.is_empty() {
                value
            } else {
                format!("{ns}.{value}")
            }
        }
        Multiname::Multiname { name, .. } | Multiname::MultinameA { name, .. } => {
            string(pool, *name)
        }
        _ => "*".into(),
    }
}

fn string(pool: &ConstantPool, index: Index<String>) -> String {
    if index.0 == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&pool.strings[index.0 as usize - 1]).into_owned()
    }
}

pub fn movie_abcs(data: &[u8]) -> Result<Vec<AbcFile>> {
    let movie = decompress(data)?;
    split_tags(&movie.body)?
        .into_iter()
        .filter(|t| matches!(t.code, 72 | 82))
        .map(|tag| {
            let body = &movie.body[tag.body_range()];
            read_abc(
                body.get(abc_block_offset(tag.code, body)..)
                    .ok_or("Truncated ABC header")?,
            )
        })
        .collect()
}

pub fn edit_movie(
    data: &[u8],
    mut edit: impl FnMut(&mut AbcFile) -> Result<bool>,
) -> Result<Vec<u8>> {
    let movie = decompress(data)?;
    let mut output = movie.body[..tags_offset(&movie.body)?].to_vec();
    for tag in split_tags(&movie.body)? {
        let payload = &movie.body[tag.body_range()];
        if matches!(tag.code, 72 | 82) {
            let offset = abc_block_offset(tag.code, payload);
            let mut abc = read_abc(payload.get(offset..).ok_or("Truncated ABC header")?)?;
            if edit(&mut abc)? {
                let mut bytes = payload[..offset].to_vec();
                Writer::new(&mut bytes)
                    .write(abc)
                    .map_err(|e| e.to_string())?;
                output.extend(write_tag_header(tag.code, bytes.len(), true));
                output.extend(bytes);
                continue;
            }
        }
        output.extend_from_slice(&movie.body[tag.start..tag.end()]);
    }
    assemble(movie.signature, movie.version, &output)
}

pub fn decode(code: &[u8]) -> Result<Vec<(usize, Op)>> {
    let mut reader = Reader::new(code);
    let mut ops = Vec::new();
    while !reader.as_slice().is_empty() {
        let offset = reader.pos(code);
        ops.push((
            offset,
            reader
                .read_op()
                .map_err(|e| format!("ABC instruction {offset}: {e}"))?,
        ));
    }
    Ok(ops)
}

fn branch(op: &mut Op) -> Option<&mut i32> {
    match op {
        Op::Jump { offset }
        | Op::IfTrue { offset }
        | Op::IfFalse { offset }
        | Op::IfEq { offset }
        | Op::IfNe { offset }
        | Op::IfLt { offset }
        | Op::IfLe { offset }
        | Op::IfGt { offset }
        | Op::IfGe { offset }
        | Op::IfNlt { offset }
        | Op::IfNle { offset }
        | Op::IfNgt { offset }
        | Op::IfNge { offset }
        | Op::IfStrictEq { offset }
        | Op::IfStrictNe { offset } => Some(offset),
        _ => None,
    }
}

pub fn rewrite(body: &mut MethodBody, ops: Vec<(usize, Op)>) -> Result<()> {
    let old = decode(&body.code)?;
    let ends: HashMap<_, _> = old
        .iter()
        .enumerate()
        .map(|(i, (start, _))| (*start, old.get(i + 1).map_or(body.code.len(), |p| p.0)))
        .collect();
    let mut positions = BTreeMap::new();
    let mut cursor = 0;
    let mut widths = Vec::new();
    for (start, op) in &ops {
        positions.entry(*start).or_insert(cursor);
        let mut bytes = Vec::new();
        Writer::new(&mut bytes)
            .write_op(op)
            .map_err(|e| e.to_string())?;
        widths.push(bytes.len());
        cursor += bytes.len();
    }
    positions.insert(body.code.len(), cursor);
    let target = |old: i64| -> Result<usize> {
        let old = usize::try_from(old).map_err(|_| "Negative ABC branch target")?;
        positions
            .get(&old)
            .copied()
            .ok_or_else(|| format!("ABC branch target {old} is not an instruction boundary"))
    };
    let mut code = Vec::new();
    for ((start, mut op), width) in ops.into_iter().zip(widths) {
        if let Some(offset) = branch(&mut op) {
            *offset =
                target(ends[&start] as i64 + *offset as i64)? as i32 - (code.len() + width) as i32;
        } else if let Op::LookupSwitch(ref mut switch) = op {
            switch.default_offset =
                target(start as i64 + switch.default_offset as i64)? as i32 - code.len() as i32;
            for offset in switch.case_offsets.iter_mut() {
                *offset = target(start as i64 + *offset as i64)? as i32 - code.len() as i32;
            }
        }
        Writer::new(&mut code)
            .write_op(&op)
            .map_err(|e| e.to_string())?;
    }
    for exception in &mut body.exceptions {
        exception.from_offset = target(exception.from_offset as i64)? as u32;
        exception.to_offset = target(exception.to_offset as i64)? as u32;
        exception.target_offset = target(exception.target_offset as i64)? as u32;
    }
    body.code = code;
    Ok(())
}

fn shift<T>(index: &mut Index<T>, offset: u32) {
    if index.0 != 0 {
        index.0 += offset;
    }
}

struct Offsets {
    ints: u32,
    uints: u32,
    doubles: u32,
    strings: u32,
    namespaces: u32,
    sets: u32,
    names: u32,
    methods: u32,
    metadata: u32,
    class: u32,
}

impl Offsets {
    fn value(&self, value: &mut DefaultValue) {
        match value {
            DefaultValue::Int(i) => shift(i, self.ints),
            DefaultValue::Uint(i) => shift(i, self.uints),
            DefaultValue::Double(i) => shift(i, self.doubles),
            DefaultValue::String(i) => shift(i, self.strings),
            DefaultValue::Namespace(i)
            | DefaultValue::Package(i)
            | DefaultValue::PackageInternal(i)
            | DefaultValue::Protected(i)
            | DefaultValue::Explicit(i)
            | DefaultValue::StaticProtected(i)
            | DefaultValue::Private(i) => shift(i, self.namespaces),
            _ => {}
        }
    }
    fn traits(&self, traits: &mut [Trait]) {
        for t in traits {
            shift(&mut t.name, self.names);
            for m in &mut t.metadata {
                m.0 += self.metadata;
            }
            match &mut t.kind {
                TraitKind::Slot {
                    type_name, value, ..
                }
                | TraitKind::Const {
                    type_name, value, ..
                } => {
                    shift(type_name, self.names);
                    if let Some(v) = value {
                        self.value(v);
                    }
                }
                TraitKind::Method { method, .. }
                | TraitKind::Getter { method, .. }
                | TraitKind::Setter { method, .. } => method.0 += self.methods,
                TraitKind::Function { function, .. } => function.0 += self.methods,
                TraitKind::Class { class, .. } => class.0 = self.class,
            }
        }
    }
    fn op(&self, op: &mut Op) {
        match op {
            Op::AsType { type_name: index }
            | Op::CallProperty { index, .. }
            | Op::CallPropLex { index, .. }
            | Op::CallPropVoid { index, .. }
            | Op::CallSuper { index, .. }
            | Op::CallSuperVoid { index, .. }
            | Op::Coerce { index }
            | Op::ConstructProp { index, .. }
            | Op::DeleteProperty { index }
            | Op::FindDef { index }
            | Op::FindProperty { index }
            | Op::FindPropStrict { index }
            | Op::GetDescendants { index }
            | Op::GetLex { index }
            | Op::GetProperty { index }
            | Op::GetSuper { index }
            | Op::InitProperty { index }
            | Op::IsType { index }
            | Op::SetProperty { index }
            | Op::SetSuper { index } => shift(index, self.names),
            Op::CallStatic { index, .. } | Op::NewFunction { index } => index.0 += self.methods,
            Op::NewClass { index } => index.0 = self.class,
            Op::Debug {
                register_name: index,
                ..
            }
            | Op::DebugFile { file_name: index }
            | Op::Dxns { index }
            | Op::PushString { value: index } => shift(index, self.strings),
            Op::PushInt { value } => shift(value, self.ints),
            Op::PushUint { value } => shift(value, self.uints),
            Op::PushDouble { value } => shift(value, self.doubles),
            Op::PushNamespace { value } => shift(value, self.namespaces),
            _ => {}
        }
    }
}

fn merge_class(
    abc: &mut AbcFile,
    mut compiled: AbcFile,
    target: usize,
    augment: bool,
) -> Result<()> {
    if compiled.instances.len() != 1 {
        return Err("A replacement source must declare exactly one class".into());
    }
    if name(&abc.constant_pool, abc.instances[target].name)
        != name(&compiled.constant_pool, compiled.instances[0].name)
    {
        return Err("Replacement class name must match the source movie".into());
    }
    let pool = &mut abc.constant_pool;
    let offsets = Offsets {
        ints: pool.ints.len() as u32,
        uints: pool.uints.len() as u32,
        doubles: pool.doubles.len() as u32,
        strings: pool.strings.len() as u32,
        namespaces: pool.namespaces.len() as u32,
        sets: pool.namespace_sets.len() as u32,
        names: pool.multinames.len() as u32,
        methods: abc.methods.len() as u32,
        metadata: abc.metadata.len() as u32,
        class: target as u32,
    };
    let cp = &mut compiled.constant_pool;
    for ns in &mut cp.namespaces {
        let (Namespace::Namespace(i)
        | Namespace::Package(i)
        | Namespace::PackageInternal(i)
        | Namespace::Protected(i)
        | Namespace::Explicit(i)
        | Namespace::StaticProtected(i)
        | Namespace::Private(i)) = ns;
        shift(i, offsets.strings);
    }
    for set in &mut cp.namespace_sets {
        for index in set {
            shift(index, offsets.namespaces);
        }
    }
    for mn in &mut cp.multinames {
        match mn {
            Multiname::QName { namespace, name } | Multiname::QNameA { namespace, name } => {
                shift(namespace, offsets.namespaces);
                shift(name, offsets.strings);
            }
            Multiname::RTQName { name } | Multiname::RTQNameA { name } => {
                shift(name, offsets.strings)
            }
            Multiname::Multiname {
                namespace_set,
                name,
            }
            | Multiname::MultinameA {
                namespace_set,
                name,
            } => {
                shift(namespace_set, offsets.sets);
                shift(name, offsets.strings);
            }
            Multiname::MultinameL { namespace_set } | Multiname::MultinameLA { namespace_set } => {
                shift(namespace_set, offsets.sets)
            }
            Multiname::TypeName {
                base_type,
                parameters,
            } => {
                shift(base_type, offsets.names);
                for p in parameters {
                    shift(p, offsets.names);
                }
            }
            _ => {}
        }
    }
    pool.ints.append(&mut cp.ints);
    pool.uints.append(&mut cp.uints);
    pool.doubles.append(&mut cp.doubles);
    pool.strings.append(&mut cp.strings);
    pool.namespaces.append(&mut cp.namespaces);
    pool.namespace_sets.append(&mut cp.namespace_sets);
    pool.multinames.append(&mut cp.multinames);
    if augment {
        let existing: HashMap<_, _> = abc.instances[target]
            .traits
            .iter()
            .chain(abc.classes[target].traits.iter())
            .map(|t| {
                (
                    name(pool, t.name),
                    pool.multinames[t.name.0 as usize - 1].clone(),
                )
            })
            .collect();
        for i in offsets.names as usize..pool.multinames.len() {
            if let Some(original) = existing.get(&name(pool, Index::new(i as u32 + 1))) {
                pool.multinames[i] = original.clone();
            }
        }
    }
    for method in &mut compiled.methods {
        shift(&mut method.name, offsets.strings);
        shift(&mut method.return_type, offsets.names);
        method.body = None;
        for param in &mut method.params {
            shift(&mut param.kind, offsets.names);
            if let Some(name) = &mut param.name {
                shift(name, offsets.strings);
            }
            if let Some(value) = &mut param.default_value {
                offsets.value(value);
            }
        }
    }
    for metadata in &mut compiled.metadata {
        shift(&mut metadata.name, offsets.strings);
        for item in &mut metadata.items {
            shift(&mut item.key, offsets.strings);
            shift(&mut item.value, offsets.strings);
        }
    }
    for body in &mut compiled.method_bodies {
        body.method.0 += offsets.methods;
        let mut ops = decode(&body.code)?;
        for (_, op) in &mut ops {
            offsets.op(op);
        }
        rewrite(body, ops)?;
        for exception in &mut body.exceptions {
            shift(&mut exception.type_name, offsets.names);
            shift(&mut exception.variable_name, offsets.names);
        }
        offsets.traits(&mut body.traits);
    }
    let mut instance = compiled.instances.remove(0);
    shift(&mut instance.name, offsets.names);
    shift(&mut instance.super_name, offsets.names);
    if let Some(ns) = &mut instance.protected_namespace {
        shift(ns, offsets.namespaces);
    }
    for interface in &mut instance.interfaces {
        shift(interface, offsets.names);
    }
    instance.init_method.0 += offsets.methods;
    offsets.traits(&mut instance.traits);
    let mut class = compiled.classes.remove(0);
    class.init_method.0 += offsets.methods;
    offsets.traits(&mut class.traits);
    if augment {
        for t in &mut instance.traits {
            let key = name(pool, t.name);
            if abc.instances[target]
                .traits
                .iter()
                .any(|old| name(pool, old.name) == key)
            {
                return Err(format!("Augmented trait {key} already exists"));
            }
            match &mut t.kind {
                TraitKind::Slot { slot_id, .. } | TraitKind::Const { slot_id, .. } => *slot_id = 0,
                _ => {}
            }
        }
        abc.instances[target].traits.extend(instance.traits);
        if !class.traits.is_empty() {
            return Err("Static augmentation is not supported".into());
        }
    } else {
        let script = abc.scripts.iter_mut().find(|s| s.traits.iter().any(|t| matches!(t.kind, TraitKind::Class { class, .. } if class.0 == target as u32))).ok_or("Replacement class has no owning script")?;
        if script.traits.len() == 1 && compiled.scripts.len() == 1 {
            script.init_method.0 = compiled.scripts[0].init_method.0 + offsets.methods;
        } else {
            if name(pool, abc.instances[target].super_name) != name(pool, instance.super_name) {
                return Err(
                    "Cannot change the superclass of a class sharing its initialization script"
                        .into(),
                );
            }
            let original = abc
                .method_bodies
                .iter()
                .find(|b| b.method == abc.instances[target].init_method)
                .ok_or("Missing original constructor")?
                .init_scope_depth;
            let replacement = compiled
                .method_bodies
                .iter()
                .find(|b| b.method == instance.init_method)
                .ok_or("Missing replacement constructor")?
                .init_scope_depth;
            for body in &mut compiled.method_bodies {
                body.init_scope_depth = (body.init_scope_depth as i64 + original as i64
                    - replacement as i64)
                    .try_into()
                    .map_err(|_| "Invalid replacement scope depth")?;
                body.max_scope_depth = (body.max_scope_depth as i64 + original as i64
                    - replacement as i64)
                    .try_into()
                    .map_err(|_| "Invalid replacement scope depth")?;
            }
        }
        abc.instances[target] = instance;
        abc.classes[target] = class;
    }
    abc.methods.extend(compiled.methods);
    abc.metadata.extend(compiled.metadata);
    abc.method_bodies.extend(compiled.method_bodies);
    Ok(())
}

pub fn replace_classes(
    data: &[u8],
    replacements: &BTreeMap<String, String>,
    dependencies: &[Vec<u8>],
) -> Result<Vec<u8>> {
    compile_classes(data, replacements, dependencies, false)
}

pub fn augment_classes(
    data: &[u8],
    replacements: &BTreeMap<String, String>,
    dependencies: &[Vec<u8>],
) -> Result<Vec<u8>> {
    compile_classes(data, replacements, dependencies, true)
}

fn compile_classes(
    data: &[u8],
    replacements: &BTreeMap<String, String>,
    dependencies: &[Vec<u8>],
    augment: bool,
) -> Result<Vec<u8>> {
    let mut types = HashMap::new();
    for bytes in std::iter::once(data).chain(dependencies.iter().map(Vec::as_slice)) {
        for abc in movie_abcs(bytes)? {
            for instance in &abc.instances {
                types.insert(
                    name(&abc.constant_pool, instance.name),
                    name(&abc.constant_pool, instance.super_name),
                );
            }
        }
    }
    let mut found = BTreeMap::new();
    let result = edit_movie(data, |abc| {
        let mut changed = false;
        for (key, source) in replacements {
            if let Some(index) = abc
                .instances
                .iter()
                .position(|i| name(&abc.constant_pool, i.name) == *key)
            {
                let compiled = as3_native::compile_sources_with_types(&[source.as_str()], &types)
                    .map_err(|e| format!("{key}: {e}"))?;
                merge_class(abc, read_abc(&compiled)?, index, augment)?;
                *found.entry(key.clone()).or_insert(0) += 1;
                changed = true;
            }
        }
        Ok(changed)
    })?;
    for key in replacements.keys() {
        if found.get(key) != Some(&1) {
            return Err(format!("Expected exactly one definition of {key}"));
        }
    }
    Ok(result)
}

fn instruction(pool: &ConstantPool, op: &Op) -> serde_json::Value {
    use serde_json::json;
    match op {
        Op::GetLocal { index } => json!(["getlocal", index]),
        Op::GetScopeObject { index } => json!(["getscopeobject", index]),
        Op::GetSlot { index } => json!(["getslot", index]),
        Op::GetLex { index } => json!(["getlex", name(pool, *index)]),
        Op::GetProperty { index } => json!(["getproperty", name(pool, *index)]),
        Op::CallProperty { index, num_args } => {
            json!(["callproperty", name(pool, *index), num_args])
        }
        Op::CallPropVoid { index, num_args } => {
            json!(["callpropvoid", name(pool, *index), num_args])
        }
        Op::ConvertB => json!(["convert_b"]),
        Op::IfFalse { offset } => json!(["iffalse", offset]),
        _ => serde_json::Value::Null,
    }
}

fn qname(pool: &mut ConstantPool, qualified: &str) -> Index<Multiname> {
    let (package, simple) = qualified.rsplit_once('.').unwrap_or(("", qualified));
    let intern = |strings: &mut Vec<Vec<u8>>, value: &str| -> Index<String> {
        if let Some(i) = strings.iter().position(|s| s == value.as_bytes()) {
            Index::new(i as u32 + 1)
        } else {
            strings.push(value.as_bytes().to_vec());
            Index::new(strings.len() as u32)
        }
    };
    let namespace = Namespace::Package(intern(&mut pool.strings, package));
    let ns = if let Some(i) = pool.namespaces.iter().position(|n| *n == namespace) {
        i + 1
    } else {
        pool.namespaces.push(namespace);
        pool.namespaces.len()
    };
    let mn = Multiname::QName {
        namespace: Index::new(ns as u32),
        name: intern(&mut pool.strings, simple),
    };
    let i = if let Some(i) = pool.multinames.iter().position(|n| *n == mn) {
        i + 1
    } else {
        pool.multinames.push(mn);
        pool.multinames.len()
    };
    Index::new(i as u32)
}

pub fn patch_method(
    data: &[u8],
    class_name: &str,
    method_name: &str,
    pattern: &str,
    replacement: &str,
) -> Result<Vec<u8>> {
    patch_method_count(data, class_name, method_name, pattern, replacement, 1)
}

pub fn patch_method_count(
    data: &[u8],
    class_name: &str,
    method_name: &str,
    pattern: &str,
    replacement: &str,
    expected_matches: usize,
) -> Result<Vec<u8>> {
    let pattern: Vec<Vec<serde_json::Value>> =
        serde_json::from_str(pattern).map_err(|e| e.to_string())?;
    let replacement: Vec<Vec<serde_json::Value>> =
        serde_json::from_str(replacement).map_err(|e| e.to_string())?;
    if pattern.is_empty() {
        return Err("Instruction pattern is empty".into());
    }
    let mut matches = 0;
    let result = edit_movie(data, |abc| {
        let Some(instance) = abc
            .instances
            .iter()
            .find(|i| name(&abc.constant_pool, i.name) == class_name)
        else {
            return Ok(false);
        };
        let method = if method_name == "$constructor" {
            instance.init_method
        } else {
            let methods: Vec<_> = instance
                .traits
                .iter()
                .filter(|t| name(&abc.constant_pool, t.name) == method_name)
                .filter_map(|t| match t.kind {
                    TraitKind::Method { method, .. }
                    | TraitKind::Getter { method, .. }
                    | TraitKind::Setter { method, .. } => Some(method),
                    _ => None,
                })
                .collect();
            if methods.len() != 1 {
                return Err(format!("Expected one method {class_name}.{method_name}"));
            }
            methods[0]
        };
        let body = abc
            .method_bodies
            .iter_mut()
            .find(|b| b.method == method)
            .ok_or("Missing method body")?;
        let mut ops = decode(&body.code)?;
        let positions: Vec<_> = ops
            .windows(pattern.len())
            .enumerate()
            .filter_map(|(i, window)| {
                window
                    .iter()
                    .zip(&pattern)
                    .all(|((_, op), expected)| {
                        let actual = instruction(&abc.constant_pool, op);
                        actual.as_array().is_some_and(|a| {
                            a.len() == expected.len()
                                && a.iter().zip(expected).all(|(a, e)| e.is_null() || a == e)
                        })
                    })
                    .then_some(i)
            })
            .collect();
        if expected_matches == 0 || positions.len() != expected_matches {
            return Err(format!(
                "Expected {expected_matches} instruction matches in {class_name}.{method_name}, found {}",
                positions.len()
            ));
        }
        if positions.windows(2).any(|p| p[1] < p[0] + pattern.len()) {
            return Err("Overlapping instruction matches".into());
        }
        for position in positions.into_iter().rev() {
            let start = ops[position].0;
            let mut new_ops = Vec::new();
            for spec in &replacement {
                let opcode = spec
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or("Missing replacement opcode")?;
                let number = |i: usize| {
                    spec.get(i)
                        .and_then(|v| v.as_u64())
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or("Missing instruction integer operand")
                };
                let identifier = |i: usize| {
                    spec.get(i)
                        .and_then(|v| v.as_str())
                        .ok_or("Missing instruction name operand")
                };
                if opcode == "keep" {
                    let index = number(1)? as usize;
                    if index >= pattern.len() {
                        return Err("Invalid keep index".into());
                    }
                    new_ops.push(ops[position + index].clone());
                    continue;
                }
                let op = match opcode {
                    "getlocal" => Op::GetLocal { index: number(1)? },
                    "getlex" => Op::GetLex {
                        index: qname(&mut abc.constant_pool, identifier(1)?),
                    },
                    "callproperty" => Op::CallProperty {
                        index: qname(&mut abc.constant_pool, identifier(1)?),
                        num_args: number(2)?,
                    },
                    "callpropvoid" => Op::CallPropVoid {
                        index: qname(&mut abc.constant_pool, identifier(1)?),
                        num_args: number(2)?,
                    },
                    _ => return Err(format!("Unsupported replacement instruction {opcode}")),
                };
                new_ops.push((start, op));
            }
            ops.splice(position..position + pattern.len(), new_ops);
        }
        rewrite(body, ops)?;
        // Added receiver calls may execute while the original expression is on the stack.
        body.max_stack += replacement.len() as u32;
        matches += 1;
        Ok(true)
    })?;
    if matches != 1 {
        return Err(format!(
            "Expected one definition of {class_name}.{method_name}"
        ));
    }
    Ok(result)
}
