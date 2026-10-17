//! Native AVM2 → ActionScript 3 decompiler for reading compiled menus (FO4/FO76/Starfield
//! Scaleform). Output is meant to be read, not recompiled: methods the structurer cannot
//! express fall back to an annotated disassembly instead of guessed code.
//!
//! Control-flow recovery follows the approach of JPEXS Free Flash Decompiler (GPL-3).

mod expr;
mod flow;

use crate::abc_edit::{decode, movie_abcs};
use crate::abc_inspect::{class_methods, find, instruction};
use expr::{Expr, Key, Locals, Printer, Stmt, quote};
use flow::Body;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use swf::avm2::types::*;

type Result<T> = std::result::Result<T, String>;

const AS3_NAMESPACE: &str = "http://adobe.com/AS3/2006/builtin";

pub(crate) struct KeyParts {
    pub name: String,
    pub attribute: bool,
    pub runtime_name: bool,
    pub runtime_ns: bool,
}

/// Name resolution for one ABC file while printing one class.
pub(crate) struct Abc<'a> {
    pub abc: &'a AbcFile,
    defined: &'a HashMap<String, String>,
    package: String,
    class: Option<usize>,
    imports: RefCell<BTreeSet<String>>,
}

impl<'a> Abc<'a> {
    fn new(abc: &'a AbcFile, defined: &'a HashMap<String, String>, class: Option<usize>) -> Self {
        let package = class.map(|c| split(&qualified(abc, abc.instances[c].name)).0).unwrap_or_default();
        Self { abc, defined, package, class, imports: RefCell::new(BTreeSet::new()) }
    }

    pub fn string(&self, index: Index<String>) -> String {
        index
            .0
            .checked_sub(1)
            .and_then(|i| self.abc.constant_pool.strings.get(i as usize))
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    }

    fn namespace(&self, index: Index<Namespace>) -> Option<&Namespace> {
        index.0.checked_sub(1).and_then(|i| self.abc.constant_pool.namespaces.get(i as usize))
    }

    pub fn namespace_uri(&self, index: Index<Namespace>) -> String {
        match self.namespace(index) {
            Some(
                Namespace::Namespace(s)
                | Namespace::Package(s)
                | Namespace::PackageInternal(s)
                | Namespace::Protected(s)
                | Namespace::Explicit(s)
                | Namespace::StaticProtected(s)
                | Namespace::Private(s),
            ) => self.string(*s),
            None => String::new(),
        }
    }

    fn multiname(&self, index: Index<Multiname>) -> Option<&Multiname> {
        index.0.checked_sub(1).and_then(|i| self.abc.constant_pool.multinames.get(i as usize))
    }

    fn import(&self, package: &str, name: &str) {
        if !package.is_empty() && package != self.package && is_identifier_path(package) {
            self.imports.borrow_mut().insert(format!("{package}.{name}"));
        }
    }

    /// The name as source spells it, recording an import for a class from another package.
    pub fn simple(&self, index: Index<Multiname>) -> String {
        match self.multiname(index) {
            None => "*".into(),
            Some(Multiname::QName { namespace, name } | Multiname::QNameA { namespace, name }) => {
                let local = self.string(*name);
                if let Some(Namespace::Package(p)) = self.namespace(*namespace) {
                    self.import(&self.string(*p), &local);
                }
                local
            }
            Some(Multiname::Multiname { name, .. } | Multiname::MultinameA { name, .. }) => {
                let local = self.string(*name);
                if let Some(package) = self.defined.get(&local) {
                    self.import(package, &local);
                }
                local
            }
            Some(Multiname::RTQName { name } | Multiname::RTQNameA { name }) => self.string(*name),
            Some(Multiname::TypeName { .. }) => self.type_name(index),
            Some(_) => "*".into(),
        }
    }

    pub fn type_name(&self, index: Index<Multiname>) -> String {
        match self.multiname(index) {
            None => "*".into(),
            Some(Multiname::TypeName { base_type, parameters }) => {
                let params: Vec<String> = parameters.iter().map(|p| self.type_name(*p)).collect();
                format!("{}.<{}>", self.simple(*base_type), params.join(", "))
            }
            Some(_) => self.simple(index),
        }
    }

    pub fn key_parts(&self, index: Index<Multiname>) -> KeyParts {
        let (name, attribute, runtime_name, runtime_ns) = match self.multiname(index) {
            Some(Multiname::QName { name, .. } | Multiname::Multiname { name, .. }) => (self.string(*name), false, false, false),
            Some(Multiname::QNameA { name, .. } | Multiname::MultinameA { name, .. }) => (self.string(*name), true, false, false),
            Some(Multiname::RTQName { name }) => (self.string(*name), false, false, true),
            Some(Multiname::RTQNameA { name }) => (self.string(*name), true, false, true),
            Some(Multiname::RTQNameL) => (String::new(), false, true, true),
            Some(Multiname::RTQNameLA) => (String::new(), true, true, true),
            Some(Multiname::MultinameL { .. }) => (String::new(), false, true, false),
            Some(Multiname::MultinameLA { .. }) => (String::new(), true, true, false),
            Some(Multiname::TypeName { .. }) => (self.type_name(index), false, false, false),
            None => ("*".into(), false, false, false),
        };
        // A class referenced through a property read still needs its import.
        if !runtime_name && !name.is_empty() {
            self.simple(index);
        }
        KeyParts { name, attribute, runtime_name, runtime_ns }
    }

    pub fn global_slot(&self, slot: u32) -> String {
        self.abc
            .scripts
            .iter()
            .flat_map(|s| &s.traits)
            .find_map(|t| match &t.kind {
                TraitKind::Slot { slot_id, .. } | TraitKind::Const { slot_id, .. } | TraitKind::Class { slot_id, .. }
                | TraitKind::Function { slot_id, .. }
                    if *slot_id == slot =>
                {
                    Some(self.simple(t.name))
                }
                _ => None,
            })
            .unwrap_or_else(|| format!("§global{slot}"))
    }

    pub fn slot_of(&self, object: &Expr, slot: u32) -> Option<String> {
        let class = self.class?;
        let traits = match object {
            Expr::This => &self.abc.instances[class].traits,
            _ => return None,
        };
        traits.iter().find_map(|t| match &t.kind {
            TraitKind::Slot { slot_id, .. } | TraitKind::Const { slot_id, .. } if *slot_id == slot => Some(self.simple(t.name)),
            _ => None,
        })
    }

    fn modifiers(&self, t: &Trait, is_static: bool) -> String {
        let mut words = Vec::new();
        if t.is_override {
            words.push("override".to_string());
        }
        if t.is_final && matches!(t.kind, TraitKind::Method { .. } | TraitKind::Getter { .. } | TraitKind::Setter { .. }) {
            words.push("final".into());
        }
        let access = match self.multiname(t.name) {
            Some(Multiname::QName { namespace, .. }) => match self.namespace(*namespace) {
                Some(Namespace::Private(_)) => "private".to_string(),
                Some(Namespace::Protected(_) | Namespace::StaticProtected(_)) => "protected".into(),
                Some(Namespace::PackageInternal(_)) => "internal".into(),
                Some(Namespace::Namespace(uri) | Namespace::Explicit(uri)) => {
                    let uri = self.string(*uri);
                    if uri == AS3_NAMESPACE {
                        "AS3".into()
                    } else {
                        let tail = uri.rsplit(['/', ':', '.']).next().unwrap_or_default();
                        if expr::is_identifier(tail) { tail.to_string() } else { "public".into() }
                    }
                }
                _ => "public".into(),
            },
            _ => "public".into(),
        };
        words.push(access);
        if is_static {
            words.push("static".into());
        }
        words.join(" ")
    }

    fn default_value(&self, value: &DefaultValue) -> String {
        let pool = &self.abc.constant_pool;
        let at = |i: u32| i.checked_sub(1).map(|i| i as usize);
        match value {
            DefaultValue::Int(i) => at(i.0).and_then(|i| pool.ints.get(i)).map_or("0".into(), |v| v.to_string()),
            DefaultValue::Uint(i) => at(i.0).and_then(|i| pool.uints.get(i)).map_or("0".into(), |v| v.to_string()),
            DefaultValue::Double(i) => at(i.0).and_then(|i| pool.doubles.get(i)).map_or("NaN".into(), |v| expr::number(*v)),
            DefaultValue::String(s) => quote(&self.string(*s)),
            DefaultValue::True => "true".into(),
            DefaultValue::False => "false".into(),
            DefaultValue::Null => "null".into(),
            DefaultValue::Undefined => "undefined".into(),
            DefaultValue::Namespace(n)
            | DefaultValue::Package(n)
            | DefaultValue::PackageInternal(n)
            | DefaultValue::Protected(n)
            | DefaultValue::Explicit(n)
            | DefaultValue::StaticProtected(n)
            | DefaultValue::Private(n) => quote(&self.namespace_uri(*n)),
        }
    }
}

fn is_identifier_path(value: &str) -> bool {
    value.split('.').all(expr::is_identifier)
}

fn qualified(abc: &AbcFile, index: Index<Multiname>) -> String {
    crate::abc_edit::name(&abc.constant_pool, index)
}

fn split(full: &str) -> (String, String) {
    match full.rsplit_once('.') {
        Some((package, name)) => (package.to_string(), name.to_string()),
        None => (String::new(), full.to_string()),
    }
}

/// A method recovered to statements, ready to print.
struct Recovered {
    stmts: Vec<Stmt>,
    locals: Locals,
    predeclared: HashSet<u32>,
    closures: Vec<(Index<Method>, Option<String>)>,
    params: Vec<String>,
    returns: String,
    failure: Option<String>,
}

fn method_body<'a>(abc: &'a AbcFile, method: Index<Method>) -> Option<&'a MethodBody> {
    let info = abc.methods.get(method.0 as usize)?;
    info.body
        .and_then(|b| abc.method_bodies.get(b.0 as usize))
        .or_else(|| abc.method_bodies.iter().find(|b| b.method == method))
}

/// `is_static` code sees its class as `this`.
fn recover(abc: &Abc, method: Index<Method>, is_static: bool) -> Recovered {
    let info = &abc.abc.methods[method.0 as usize];
    let body = method_body(abc.abc, method);
    let ops = body.map(|b| decode(&b.code)).transpose().ok().flatten().unwrap_or_default();
    // Register names: parameters, then names debug builds record.
    let mut names = HashMap::new();
    let count = info.params.len() as u32;
    for (i, p) in info.params.iter().enumerate() {
        let name = p.name.filter(|_| info.flags.contains(MethodFlags::HAS_PARAM_NAMES)).map(|n| abc.string(n));
        names.insert(i as u32 + 1, name.filter(|n| expr::is_identifier(n)).unwrap_or_else(|| format!("param{}", i + 1)));
    }
    let mut extra = count;
    if info.flags.contains(MethodFlags::NEED_REST) {
        extra += 1;
        names.insert(extra, "rest".into());
    } else if info.flags.contains(MethodFlags::NEED_ARGUMENTS) {
        extra += 1;
        names.insert(extra, "arguments".into());
    }
    for (_, op) in &ops {
        if let Op::Debug { is_local_register: true, register_name, register } = op {
            let name = abc.string(*register_name);
            if expr::is_identifier(&name) {
                names.insert(*register as u32 + 1, name);
            }
        }
    }
    let predeclared: HashSet<u32> = (0..=extra).collect();
    let returns = abc.type_name(info.return_type);
    let locals = Locals { names, ..Locals::default() };
    let mut recovered = Recovered { stmts: vec![], locals, predeclared, closures: vec![], params: vec![], returns, failure: None };
    let Some(body) = body else {
        recovered.params = signature(abc, info, &recovered.locals.names, extra);
        recovered.failure = Some("native method without a body".into());
        return recovered;
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut flow = Body::new(abc, body, ops, recovered.locals.names.clone(), extra);
        if is_static && let Some(class) = abc.class {
            flow.this = Expr::Name(split(&qualified(abc.abc, abc.abc.instances[class].name)).1);
        }
        let stmts = flow.structure()?;
        // A register's type: its conversion, else the one type every stored value has.
        let mut types = std::mem::take(&mut flow.types);
        for (r, kinds) in &flow.stored {
            if let Some(Some(first)) = kinds.first()
                && kinds.iter().all(|k| k.as_ref() == Some(first))
            {
                types.entry(*r).or_insert_with(|| first.clone());
            }
        }
        Ok::<_, String>((stmts, flow.names.into_inner(), types, flow.closures.into_inner()))
    }));
    match result {
        Ok(Ok((mut stmts, names, types, closures))) => {
            if matches!(stmts.last(), Some(Stmt::Return(None))) {
                stmts.pop();
            }
            // Activation slots are the method's variables, except the parameters copied there.
            let params: HashSet<&String> = (1..=extra).filter_map(|r| names.get(&r)).collect();
            let vars = body
                .traits
                .iter()
                .filter_map(|t| match &t.kind {
                    TraitKind::Slot { type_name, .. } => Some((abc.simple(t.name), abc.type_name(*type_name))),
                    _ => None,
                })
                .filter(|(name, _)| !params.contains(name))
                .collect();
            recovered.stmts = stmts;
            recovered.locals = Locals { names, types, vars };
            recovered.closures = closures;
        }
        Ok(Err(reason)) => recovered.failure = Some(reason),
        Err(_) => recovered.failure = Some("internal decompiler error".into()),
    }
    // Parameters the method copies into its activation take that slot's name.
    recovered.params = signature(abc, info, &recovered.locals.names, extra);
    recovered
}

fn signature(abc: &Abc, info: &Method, names: &HashMap<u32, String>, extra: u32) -> Vec<String> {
    let mut params = Vec::new();
    let optional_from = info.params.len() - info.params.iter().filter(|p| p.default_value.is_some()).count();
    for (i, p) in info.params.iter().enumerate() {
        let mut text = format!("{}:{}", names[&(i as u32 + 1)], abc.type_name(p.kind));
        if i >= optional_from
            && let Some(v) = &p.default_value
        {
            text.push_str(&format!(" = {}", abc.default_value(v)));
        }
        params.push(text);
    }
    if info.flags.contains(MethodFlags::NEED_REST) {
        params.push(format!("... {}", names[&extra]));
    }
    params
}

fn disassembly(abc: &AbcFile, method: Index<Method>) -> Vec<String> {
    let Some(body) = method_body(abc, method) else { return vec![] };
    let Ok(ops) = decode(&body.code) else { return vec!["(undecodable bytecode)".into()] };
    ops.iter()
        .enumerate()
        .map(|(i, (offset, op))| {
            let next = ops.get(i + 1).map_or(body.code.len(), |p| p.0);
            format!("{offset:>5}  {}", instruction(abc, op, *offset, next))
        })
        .collect()
}

fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// Prints a recovered body at `depth`, expanding closures in place.
fn print_body(abc: &Abc, method: Index<Method>, recovered: &Recovered, depth: usize) -> String {
    if let Some(reason) = &recovered.failure {
        let mut text = format!("{}// Decompilation failed: {reason}\n", indent(depth));
        for line in disassembly(abc.abc, method) {
            text.push_str(&format!("{}// {line}\n", indent(depth)));
        }
        return text;
    }
    let mut printer = Printer::new(&recovered.locals, recovered.predeclared.clone(), depth);
    printer.stmts(&recovered.stmts);
    let text = printer.finish();
    if recovered.closures.is_empty() {
        return text;
    }
    let mut out = String::new();
    for line in text.lines() {
        let mut line = line.to_string();
        for (n, (closure, name)) in recovered.closures.iter().enumerate() {
            let marker = format!("§closure{n}§");
            if line.contains(&marker) {
                let level = line.len() - line.trim_start().len();
                line = line.replace(&marker, &function_text(abc, *closure, name.as_deref(), level / 4));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn function_text(abc: &Abc, method: Index<Method>, name: Option<&str>, depth: usize) -> String {
    let recovered = recover(abc, method, false);
    let body = print_body(abc, method, &recovered, depth + 1);
    let name = name.map(|n| format!(" {n}")).unwrap_or_default();
    format!(
        "function{name}({}):{}\n{}{{\n{body}{}}}",
        recovered.params.join(", "),
        recovered.returns,
        indent(depth),
        indent(depth)
    )
}

/// Statements before `super()` that set this class's own fields are its field initializers,
/// when their values need nothing of the instance beyond its own fields (no methods, locals
/// or closures). `owner` is how the code names the object holding the fields: `this`, or the
/// class itself in a static initializer.
fn take_initializers(stmts: &mut Vec<Stmt>, fields: &HashSet<String>, owner: &Expr) -> HashMap<String, Expr> {
    let is_static = *owner != Expr::This;
    let mut found = HashMap::new();
    let mut keep = Vec::new();
    let mut leading = true;
    for s in stmts.drain(..) {
        if leading {
            if let Stmt::Expr(Expr::Assign(target, value)) = &s {
                let name = match target.stripped() {
                    Expr::Prop(obj, Key::Name(n)) if obj.stripped() == owner => Some(n),
                    Expr::Name(n) if is_static => Some(n),
                    _ => None,
                };
                let own_fields_masked = value.map(&|e| match e {
                    Expr::Prop(obj, Key::Name(n)) if obj.stripped() == owner && fields.contains(n) => {
                        Some(Expr::Lit(expr::Lit::Null))
                    }
                    _ => None,
                });
                let needs_instance = own_fields_masked.any(&|e| {
                    matches!(e, Expr::This | Expr::Local(_) | Expr::Super(_) | Expr::Function(_) | Expr::Activation | Expr::In(_))
                });
                if let Some(n) = name
                    && fields.contains(n)
                    && !found.contains_key(n)
                    && !needs_instance
                {
                    found.insert(n.clone(), (**value).clone());
                    continue;
                }
            }
            if matches!(&s, Stmt::Expr(Expr::Call(callee, _)) if matches!(callee.stripped(), Expr::Raw(r) if r == "super")) || is_static {
                leading = is_static;
            }
            if !matches!(&s, Stmt::Expr(Expr::Assign(..))) {
                leading = false;
            }
        }
        keep.push(s);
    }
    *stmts = keep;
    found
}

fn render_class(abc: &Abc, index: usize) -> String {
    let instance = &abc.abc.instances[index];
    let class = &abc.abc.classes[index];
    let (package, name) = split(&qualified(abc.abc, instance.name));
    let mut members = String::new();
    let fields_of = |traits: &[Trait]| -> HashSet<String> {
        traits
            .iter()
            .filter(|t| matches!(t.kind, TraitKind::Slot { .. } | TraitKind::Const { .. }))
            .map(|t| abc.simple(t.name))
            .collect()
    };
    let mut cinit = recover(abc, class.init_method, true);
    let static_values = take_initializers(&mut cinit.stmts, &fields_of(&class.traits), &Expr::Name(name.clone()));
    let mut iinit = recover(abc, instance.init_method, false);
    let instance_values = take_initializers(&mut iinit.stmts, &fields_of(&instance.traits), &Expr::This);
    // Fields.
    for (traits, values, recovered, is_static) in
        [(&class.traits, &static_values, &cinit, true), (&instance.traits, &instance_values, &iinit, false)]
    {
        for t in traits.iter() {
            let (keyword, kind, value) = match &t.kind {
                TraitKind::Slot { type_name, value, .. } => ("var", type_name, value),
                TraitKind::Const { type_name, value, .. } => ("const", type_name, value),
                _ => continue,
            };
            let field = abc.simple(t.name);
            let mut line = format!("{}{} {keyword} {field}:{}", indent(2), abc.modifiers(t, is_static), abc.type_name(*kind));
            if let Some(v) = values.get(&field) {
                let mut printer = Printer::new(&recovered.locals, recovered.predeclared.clone(), 0);
                line.push_str(&format!(" = {}", printer.expr(v, 3)));
            } else if let Some(v) = value
                && !matches!(v, DefaultValue::Undefined)
            {
                line.push_str(&format!(" = {}", abc.default_value(v)));
            }
            members.push_str(&line);
            members.push_str(";\n");
        }
    }
    if !members.is_empty() {
        members.push('\n');
    }
    // A static initializer with statements beyond field values.
    if !cinit.stmts.is_empty() || cinit.failure.is_some() {
        members.push_str(&format!("{}// static initializer\n{}{{\n", indent(2), indent(2)));
        members.push_str(&print_body(abc, class.init_method, &cinit, 3));
        members.push_str(&format!("{}}}\n\n", indent(2)));
    }
    if !instance.is_interface {
        members.push_str(&format!("{}public function {name}({})\n{}{{\n", indent(2), iinit.params.join(", "), indent(2)));
        members.push_str(&print_body(abc, instance.init_method, &iinit, 3));
        members.push_str(&format!("{}}}\n\n", indent(2)));
    }
    for (traits, is_static) in [(&instance.traits, false), (&class.traits, true)] {
        for t in traits.iter() {
            let (prefix, method) = match &t.kind {
                TraitKind::Method { method, .. } => ("", *method),
                TraitKind::Getter { method, .. } => ("get ", *method),
                TraitKind::Setter { method, .. } => ("set ", *method),
                TraitKind::Function { function, .. } => ("", *function),
                _ => continue,
            };
            let recovered = recover(abc, method, is_static);
            let head = format!(
                "function {prefix}{}({}):{}",
                abc.simple(t.name),
                recovered.params.join(", "),
                recovered.returns
            );
            if instance.is_interface {
                members.push_str(&format!("{}{head};\n", indent(2)));
                continue;
            }
            members.push_str(&format!("{}{} {head}\n{}{{\n", indent(2), abc.modifiers(t, is_static), indent(2)));
            members.push_str(&print_body(abc, method, &recovered, 3));
            members.push_str(&format!("{}}}\n\n", indent(2)));
        }
    }
    let mut head = vec!["public".to_string()];
    if instance.is_final {
        head.push("final".into());
    }
    if !instance.is_sealed && !instance.is_interface {
        head.push("dynamic".into());
    }
    head.push(if instance.is_interface { "interface".into() } else { "class".into() });
    head.push(name.clone());
    let parent = abc.type_name(instance.super_name);
    if !instance.is_interface && parent != "*" && parent != "Object" {
        head.push(format!("extends {parent}"));
    }
    let interfaces: Vec<String> = instance.interfaces.iter().map(|i| abc.type_name(*i)).collect();
    if !interfaces.is_empty() {
        head.push(format!("{} {}", if instance.is_interface { "extends" } else { "implements" }, interfaces.join(", ")));
    }
    let mut text = format!("package {package}\n{{\n");
    let imports = abc.imports.borrow();
    for import in imports.iter() {
        text.push_str(&format!("{}import {import};\n", indent(1)));
    }
    if !imports.is_empty() {
        text.push('\n');
    }
    text.push_str(&format!("{}{}\n{}{{\n", indent(1), head.join(" "), indent(1)));
    text.push_str(members.trim_end());
    text.push('\n');
    text.push_str(&format!("{}}}\n}}\n", indent(1)));
    text.replace("package \n", "package\n")
}

fn defined_classes(abcs: &[AbcFile]) -> HashMap<String, String> {
    let mut defined = HashMap::new();
    for abc in abcs {
        for instance in &abc.instances {
            let (package, name) = split(&qualified(abc, instance.name));
            defined.insert(name, package);
        }
    }
    defined
}

/// One class as an AS3 source file.
pub fn decompile_class(data: &[u8], class: &str) -> Result<String> {
    let abcs = movie_abcs(data)?;
    let defined = defined_classes(&abcs);
    let found = find(&abcs, class)?;
    let abc = Abc::new(found.abc, &defined, Some(found.index));
    Ok(render_class(&abc, found.index))
}

/// One method of a class (`name`, `get name`, `constructor`, `static initializer`, ...).
pub fn decompile_method(data: &[u8], class: &str, method: &str) -> Result<String> {
    let abcs = movie_abcs(data)?;
    let defined = defined_classes(&abcs);
    let found = find(&abcs, class)?;
    let abc = Abc::new(found.abc, &defined, Some(found.index));
    let selected: Vec<_> = class_methods(found.abc, found.index)
        .into_iter()
        .filter(|(label, _)| label == method || label.rsplit(' ').next() == Some(method))
        .collect();
    if selected.is_empty() {
        return Err(format!("Class '{class}' has no method '{method}'"));
    }
    let mut text = String::new();
    for (label, id) in selected {
        let recovered = recover(&abc, id, label.starts_with("static"));
        let params = recovered.params.join(", ");
        let head = match label.as_str() {
            "constructor" => {
                let name = split(&qualified(found.abc, found.abc.instances[found.index].name)).1;
                format!("function {name}({params})")
            }
            "static initializer" => "// static initializer".to_string(),
            _ => match label.strip_prefix("static ") {
                Some(rest) => format!("static function {rest}({params}):{}", recovered.returns),
                None => format!("function {label}({params}):{}", recovered.returns),
            },
        };
        text.push_str(&format!("{head}\n{{\n"));
        text.push_str(&print_body(&abc, id, &recovered, 1));
        text.push_str("}\n");
    }
    Ok(text)
}

/// Every class in the movie: `(qualified name, source)`, plus how many methods fell back to
/// disassembly.
pub fn decompile_movie(data: &[u8]) -> Result<(Vec<(String, String)>, usize)> {
    let abcs = movie_abcs(data)?;
    let defined = defined_classes(&abcs);
    let mut out = Vec::new();
    let mut failures = 0;
    for file in &abcs {
        for index in 0..file.instances.len() {
            let abc = Abc::new(file, &defined, Some(index));
            let source = render_class(&abc, index);
            failures += source.matches("// Decompilation failed:").count();
            out.push((qualified(file, file.instances[index].name), source));
        }
    }
    Ok((out, failures))
}
