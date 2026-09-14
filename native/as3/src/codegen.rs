//! Lowering from the AST to an ABC block.
//!
//! Written against the *AVM2 Overview*, not from another compiler's source.
//! Structural choices that could be checked against `WeaponCND.swf` (a
//! HUDFramework widget the engine loads) were, and are marked below.
//!
//! Anything that cannot be emitted completely is refused with a diagnostic. A
//! class that misses an interface method or drops a method fails silently in the
//! player, which is worse than a compile error.

use std::collections::{HashMap, HashSet};

use crate::abc::code::{CodeBuilder, Op};
use crate::abc::file::*;
use crate::abc::pool::{ConstantPool, NsKind};
use crate::ast::*;
use crate::diag::{Diagnostic, Result, Span, Stage};
use crate::types::{TOP_LEVEL_TYPES, builtin_super, split_qualified};

/// Depth of the scope stack once the script initialiser has pushed the global
/// object, before any ancestor is pushed.
const SCRIPT_INIT_SCOPE_DEPTH: u32 = 1;
const GLOBAL_SCOPE_DEPTH: u32 = SCRIPT_INIT_SCOPE_DEPTH + 1;

/// Resolves written type names to fully-qualified ones.
struct Resolver {
    imports: HashMap<String, String>,
    wildcards: Vec<String>,
    /// Simple name → qualified name, for classes declared in this same file.
    declared: HashMap<String, String>,
}

impl Resolver {
    fn build(package: &Package) -> Result<Self> {
        let mut imports = HashMap::new();
        let mut wildcards = Vec::new();
        for import in &package.imports {
            if import.wildcard {
                wildcards.push(import.name.joined());
                continue;
            }
            let simple = import.name.last().to_string();
            let pkg = import.name.package();
            if let Some(previous) = imports.insert(simple.clone(), pkg.clone())
                && previous != pkg
            {
                return Err(Diagnostic::new(
                    Stage::Codegen,
                    format!(
                        "`{simple}` is imported from both `{previous}` and `{pkg}`; \
                         qualify the name to disambiguate"
                    ),
                    import.span,
                ));
            }
        }
        let mut declared = HashMap::new();
        for class in &package.classes {
            let qualified = if package.name.is_empty() {
                class.name.clone()
            } else {
                format!("{}.{}", package.name, class.name)
            };
            declared.insert(class.name.clone(), qualified);
        }
        Ok(Self {
            imports,
            wildcards,
            declared,
        })
    }

    fn resolve(&self, name: &DottedName) -> Result<String> {
        if name.parts.len() > 1 {
            return Ok(name.joined());
        }
        let simple = name.last();
        if let Some(q) = self.declared.get(simple) {
            return Ok(q.clone());
        }
        if let Some(pkg) = self.imports.get(simple) {
            return Ok(if pkg.is_empty() {
                simple.to_string()
            } else {
                format!("{pkg}.{simple}")
            });
        }
        if TOP_LEVEL_TYPES.contains(&simple) {
            return Ok(simple.to_string());
        }
        let hint = if self.wildcards.is_empty() {
            format!("add `import <package>.{simple};` or write the name in full")
        } else {
            format!(
                "it may come from the wildcard import{} {}, which this phase cannot \
                 expand without a type registry — import `{simple}` by name instead",
                if self.wildcards.len() == 1 { "" } else { "s" },
                self.wildcards
                    .iter()
                    .map(|w| format!("`{w}.*`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Err(Diagnostic::unsupported(
            format!("cannot resolve type `{simple}`: {hint}"),
            name.span,
        ))
    }
}

/// Ancestors of `qualified`, from `Object` down to and including `qualified`.
///
/// The script initialiser pushes one scope per entry, so the length of this
/// list *is* the class's captured scope depth minus the global scope.
fn ancestor_chain(
    qualified: &str,
    declared: &HashMap<String, ClassDecl>,
    resolver: &Resolver,
    span: Span,
) -> Result<Vec<String>> {
    let mut chain = Vec::new();
    let mut at = qualified.to_string();
    loop {
        if chain.len() > 64 {
            return Err(Diagnostic::codegen(
                format!("inheritance cycle reaching `{at}`"),
                span,
            ));
        }
        chain.push(at.clone());
        if at == "Object" {
            break;
        }
        let next = if let Some(sup) = builtin_super(&at) {
            sup.to_string()
        } else if let Some(decl) = declared.get(&at) {
            match &decl.extends {
                Some(name) => resolver.resolve(name)?,
                None => "Object".to_string(),
            }
        } else {
            return Err(Diagnostic::unsupported(
                format!(
                    "base class `{at}` has no known ancestry. A class's script initialiser \
                     must push one scope per ancestor, so the chain has to be known before \
                     the class can be emitted — extend a type this phase knows, or declare \
                     the base class in the same file"
                ),
                span,
            ));
        };
        at = next;
    }
    chain.reverse();
    Ok(chain)
}

/// Order classes so that every type a class depends on is already published.
///
/// `newclass` resolves a class's base and its interface list at the moment it
/// runs, so a type must be defined before anything that extends or implements
/// it. Source order is not required to satisfy that, and getting it wrong fails
/// at load time rather than here.
fn definition_order<'a>(
    entries: &[(usize, &'a ClassDecl)],
    packages: &[&Package],
    resolvers: &[Resolver],
) -> Result<Vec<(usize, &'a ClassDecl)>> {
    let names: HashSet<String> = entries
        .iter()
        .map(|&(file, decl)| qualify(packages[file], &decl.name))
        .collect();

    // Only types declared here can be undefined at the wrong moment; anything
    // else is already resolvable from the runtime's global scope.
    let mut deps: Vec<Vec<String>> = Vec::with_capacity(entries.len());
    for &(file, decl) in entries {
        let own = qualify(packages[file], &decl.name);
        let mut needs = Vec::new();
        for name in decl.extends.iter().chain(decl.implements.iter()) {
            let qualified = resolvers[file].resolve(name)?;
            if names.contains(&qualified) && qualified != own {
                needs.push(qualified);
            }
        }
        deps.push(needs);
    }

    let mut ordered = Vec::with_capacity(entries.len());
    let mut placed: HashSet<String> = HashSet::new();
    while ordered.len() < entries.len() {
        let mut progressed = false;
        for (index, &(file, decl)) in entries.iter().enumerate() {
            let own = qualify(packages[file], &decl.name);
            if placed.contains(&own) {
                continue;
            }
            if deps[index].iter().all(|d| placed.contains(d)) {
                placed.insert(own);
                ordered.push((file, decl));
                progressed = true;
            }
        }
        if !progressed {
            let stuck: Vec<String> = entries
                .iter()
                .map(|&(file, decl)| qualify(packages[file], &decl.name))
                .filter(|n| !placed.contains(n))
                .collect();
            return Err(Diagnostic::codegen(
                format!(
                    "circular dependency between {} — a type cannot be defined before itself",
                    stuck.join(", ")
                ),
                entries[0].1.span,
            ));
        }
    }
    Ok(ordered)
}

fn qualify(package: &Package, name: &str) -> String {
    if package.name.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", package.name, name)
    }
}

/// Verify that a class satisfies every interface it declares.
///
/// AVM2 builds the interface method table from the class's own method traits,
/// so an `implements` without a matching public method produces a widget that
/// loads and then does nothing.
///
/// The interface must be declared in the same compilation unit, as
/// `WeaponCND.swf` does with its own copy of `hudframework.IHUDWidget`; an
/// interface with unknown members cannot be checked.
fn check_implements(
    decl: &ClassDecl,
    declared: &HashMap<String, ClassDecl>,
    resolver: &Resolver,
) -> Result<()> {
    for name in &decl.implements {
        let qualified = resolver.resolve(name)?;
        let Some(interface) = declared.get(&qualified) else {
            return Err(Diagnostic::unsupported(
                format!(
                    "interface `{qualified}` is not declared in this file, so the methods \
                     `{}` must implement cannot be checked. Compile the interface's source \
                     into the same file — a shipping HUDFramework widget does exactly that \
                     with `hudframework.IHUDWidget`",
                    decl.name
                ),
                name.span,
            ));
        };
        if !interface.is_interface {
            return Err(Diagnostic::parse(
                format!("`{qualified}` is a class, not an interface"),
                name.span,
            ));
        }
        for member in &interface.members {
            let Member::Function(required) = member else {
                continue;
            };
            let found = decl.members.iter().find_map(|m| match m {
                Member::Function(f) if f.name == required.name => Some(f),
                _ => None,
            });
            let Some(found) = found else {
                return Err(Diagnostic::new(
                    Stage::Codegen,
                    format!(
                        "`{}` declares `implements {qualified}` but does not implement \
                         `{}({})`",
                        decl.name,
                        required.name,
                        required
                            .sig
                            .params
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    decl.span,
                ));
            };
            if found.sig.params.len() != required.sig.params.len() {
                return Err(Diagnostic::new(
                    Stage::Codegen,
                    format!(
                        "`{}.{}` takes {} parameter(s) but `{qualified}` requires {}",
                        decl.name,
                        found.name,
                        found.sig.params.len(),
                        required.sig.params.len()
                    ),
                    found.span,
                ));
            }
            if !matches!(found.modifiers.visibility, None | Some(Visibility::Public)) {
                return Err(Diagnostic::new(
                    Stage::Codegen,
                    format!(
                        "`{}.{}` must be public to satisfy `{qualified}`",
                        decl.name, found.name
                    ),
                    found.span,
                ));
            }
        }
    }
    Ok(())
}

/// A class after name resolution, before any code has been emitted.
struct ClassPlan<'a> {
    decl: &'a ClassDecl,
    resolver: &'a Resolver,
    /// Interned QName of the class itself.
    name: u32,
    /// Interned QName of the direct superclass, or 0 for an interface.
    super_name: u32,
    /// Interned QNames of every ancestor, `Object` first.
    chain: Vec<u32>,
    interfaces: Vec<u32>,
    flags: u8,
    /// Scope depth captured at `newclass`.
    captured: u32,
    /// Namespace instance method traits are declared in.
    member_ns: u32,
    /// Names of every member declared on this class, for resolving a bare
    /// identifier in a method body to `this.<name>`.
    members: HashSet<String>,
}

pub fn compile_unit(unit: &CompilationUnit) -> Result<Vec<u8>> {
    compile_units(std::slice::from_ref(unit))
}

/// Compile several parsed source files into one ABC block.
///
/// A widget needs this: its document class lives in the unnamed package while
/// the interface it implements lives in `hudframework`, and AS3 allows only one
/// package per file. `WeaponCND.swf` is built the same way — one ABC holding
/// both `hudframework.IHUDWidget` and `Main`.
pub fn compile_units(units: &[CompilationUnit]) -> Result<Vec<u8>> {
    let mut packages: Vec<&Package> = Vec::new();
    for unit in units {
        if unit.packages.len() != 1 {
            let span = unit.packages.get(1).map(|p| p.span).unwrap_or_default();
            return Err(Diagnostic::unsupported(
                "a source file must declare exactly one package",
                span,
            ));
        }
        packages.push(&unit.packages[0]);
    }
    if packages.iter().all(|p| p.classes.is_empty()) {
        let span = packages.first().map(|p| p.span).unwrap_or_default();
        return Err(Diagnostic::unsupported(
            "no classes are declared, so there is nothing to emit",
            span,
        ));
    }

    // Imports are per-file, so each package resolves names with its own table.
    let resolvers: Vec<Resolver> = packages
        .iter()
        .map(|p| Resolver::build(p))
        .collect::<Result<Vec<_>>>()?;

    let mut declared: HashMap<String, ClassDecl> = HashMap::new();
    for package in &packages {
        for class in &package.classes {
            declared.insert(qualify(package, &class.name), class.clone());
        }
    }

    // Every class paired with the file it came from, so it keeps that file's
    // import table.
    let entries: Vec<(usize, &ClassDecl)> = packages
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.classes.iter().map(move |c| (i, c)))
        .collect();

    for &(file, decl) in &entries {
        check_implements(decl, &declared, &resolvers[file])?;
    }

    let ordered = definition_order(&entries, &packages, &resolvers)?;

    let mut pool = ConstantPool::new();
    let mut plans = Vec::new();
    for &(file, decl) in &ordered {
        plans.push(plan_class(
            &mut pool,
            &resolvers[file],
            &declared,
            packages[file],
            decl,
        )?);
    }

    let mut abc = AbcFile::new(pool);
    // Method indices are assigned as methods are created; a class records the
    // indices it owns so the script initialiser and traits can refer to them.
    let mut class_bodies: Vec<ClassEmission> = Vec::new();
    for plan in &plans {
        class_bodies.push(emit_class(&mut abc, plan)?);
    }

    for (index, plan) in plans.iter().enumerate() {
        let e = &class_bodies[index];
        abc.instances.push(InstanceInfo {
            name: plan.name,
            super_name: plan.super_name,
            flags: plan.flags,
            protected_ns: None,
            interfaces: plan.interfaces.clone(),
            iinit: e.iinit,
            traits: e.instance_traits.clone(),
        });
    }
    for e in &class_bodies {
        abc.classes.push(ClassInfo {
            cinit: e.cinit,
            traits: Vec::new(),
        });
    }

    let unit_span = packages[0].span;
    let script_init = abc.methods.len() as u32;
    abc.methods.push(MethodInfo::default());
    abc.scripts.push(ScriptInfo {
        init: script_init,
        traits: plans
            .iter()
            .enumerate()
            .map(|(index, plan)| Trait {
                name: plan.name,
                kind: TraitKind::Class {
                    slot_id: index as u32 + 1,
                    class_index: index as u32,
                },
            })
            .collect(),
    });

    let script_code = script_init_code(&plans);
    let stats = script_code
        .analyze(SCRIPT_INIT_SCOPE_DEPTH, 1)
        .map_err(|e| Diagnostic::codegen(e, unit_span))?;
    abc.bodies.push(MethodBody {
        method: script_init,
        max_stack: stats.max_stack,
        local_count: stats.local_count,
        init_scope_depth: SCRIPT_INIT_SCOPE_DEPTH,
        max_scope_depth: stats.max_scope_depth,
        code: script_code
            .assemble()
            .map_err(|e| Diagnostic::codegen(e, unit_span))?,
        traits: Vec::new(),
    });

    abc.write().map_err(|e| Diagnostic::codegen(e, unit_span))
}

fn plan_class<'a>(
    pool: &mut ConstantPool,
    resolver: &'a Resolver,
    declared: &HashMap<String, ClassDecl>,
    package: &Package,
    decl: &'a ClassDecl,
) -> Result<ClassPlan<'a>> {
    match decl.modifiers.visibility {
        None | Some(Visibility::Public) => {}
        Some(_) => {
            return Err(Diagnostic::unsupported(
                "only `public` (or unqualified) top-level types are emitted in this phase",
                decl.modifiers.span,
            ));
        }
    }

    let qualified = if package.name.is_empty() {
        decl.name.clone()
    } else {
        format!("{}.{}", package.name, decl.name)
    };

    let super_qualified = if decl.is_interface {
        None
    } else {
        Some(match &decl.extends {
            Some(name) => resolver.resolve(name)?,
            None => "Object".to_string(),
        })
    };

    // The superclass is interned before the class being declared: the runtime
    // needs it first (`getlex` the base, then `newclass`).
    let super_name = match &super_qualified {
        Some(s) => {
            let (pkg, simple) = split_qualified(s);
            pool.qname(NsKind::PackageNamespace, pkg, simple)
        }
        None => 0,
    };

    // An interface pushes no ancestor scopes at all — `WeaponCND.swf` builds its
    // own `IHUDWidget` with a bare `pushnull; newclass` at scope depth 2, while
    // its `Main` pushes all seven of `MovieClip`'s ancestors to reach depth 9.
    let chain = match &super_qualified {
        None => Vec::new(),
        Some(s) => ancestor_chain(s, declared, resolver, decl.span)?
            .iter()
            .map(|q| {
                let (pkg, simple) = split_qualified(q);
                pool.qname(NsKind::PackageNamespace, pkg, simple)
            })
            .collect(),
    };
    let captured = GLOBAL_SCOPE_DEPTH + chain.len() as u32;

    let mut interfaces = Vec::new();
    for name in &decl.implements {
        let q = resolver.resolve(name)?;
        let (pkg, simple) = split_qualified(&q);
        interfaces.push(pool.qname(NsKind::PackageNamespace, pkg, simple));
    }

    let name = pool.qname(NsKind::PackageNamespace, &package.name, &decl.name);

    // Interface methods live in a namespace of their own, spelled
    // `package:Interface` with kind `Namespace` — checked against
    // `WeaponCND.swf`, whose `IHUDWidget.processMessage` trait sits in
    // `Namespace("hudframework:IHUDWidget")`. A class's own public members
    // instead use the public namespace, which is `PackageNamespace("")`.
    let member_ns = if decl.is_interface {
        let (pkg, simple) = split_qualified(&qualified);
        pool.namespace(NsKind::Namespace, &format!("{pkg}:{simple}"))
    } else {
        pool.namespace(NsKind::PackageNamespace, "")
    };

    let mut flags = if decl.is_interface {
        CLASS_SEALED | CLASS_INTERFACE
    } else if decl.modifiers.is_dynamic {
        0
    } else {
        // AS3 classes are sealed unless declared `dynamic`. `WeaponCND.swf`'s
        // document class is sealed (flags 0x09), which settles the question of
        // what a working widget actually ships.
        CLASS_SEALED
    };
    if decl.modifiers.is_final {
        flags |= CLASS_FINAL;
    }

    let mut members = HashSet::new();
    for member in &decl.members {
        match member {
            Member::Function(f) => members.insert(f.name.clone()),
            Member::Var(v) => members.insert(v.name.clone()),
        };
    }

    Ok(ClassPlan {
        decl,
        resolver,
        name,
        super_name,
        chain,
        interfaces,
        flags,
        captured,
        member_ns,
        members,
    })
}

struct ClassEmission {
    iinit: u32,
    cinit: u32,
    instance_traits: Vec<Trait>,
}

fn emit_class(abc: &mut AbcFile, plan: &ClassPlan<'_>) -> Result<ClassEmission> {
    let decl = plan.decl;
    let mut constructor: Option<&FunctionDecl> = None;
    let mut methods: Vec<&FunctionDecl> = Vec::new();

    for member in &decl.members {
        match member {
            Member::Function(f) if !decl.is_interface && f.name == decl.name => {
                if constructor.is_some() {
                    return Err(Diagnostic::parse(
                        format!("class `{}` declares more than one constructor", decl.name),
                        f.span,
                    ));
                }
                constructor = Some(f);
            }
            Member::Function(f) => {
                if f.accessor != Accessor::None {
                    return Err(Diagnostic::unsupported(
                        format!(
                            "accessor `{}` needs a getter/setter trait pair, which this \
                             phase does not emit",
                            f.name
                        ),
                        f.span,
                    ));
                }
                if f.modifiers.is_static {
                    return Err(Diagnostic::unsupported(
                        format!("static member `{}` is not emitted in this phase", f.name),
                        f.span,
                    ));
                }
                if f.modifiers.is_override {
                    return Err(Diagnostic::unsupported(
                        format!(
                            "`override {}` needs the trait OVERRIDE attribute and a check \
                             against the base class, which this phase does not do",
                            f.name
                        ),
                        f.span,
                    ));
                }
                match f.modifiers.visibility {
                    None | Some(Visibility::Public) => {}
                    Some(_) => {
                        return Err(Diagnostic::unsupported(
                            format!(
                                "only `public` members are emitted in this phase; `{}` is not",
                                f.name
                            ),
                            f.span,
                        ));
                    }
                }
                methods.push(f);
            }
            Member::Var(v) => {
                return Err(Diagnostic::unsupported(
                    format!(
                        "field `{}` needs a slot trait, which this phase does not emit",
                        v.name
                    ),
                    v.span,
                ));
            }
        }
    }

    // An interface's methods are declarations only, and its instance
    // initialiser has no body at all — both checked against `WeaponCND.swf`.
    if decl.is_interface {
        let mut traits = Vec::new();
        for f in &methods {
            if f.body.is_some() {
                return Err(Diagnostic::parse(
                    format!("interface method `{}` may not have a body", f.name),
                    f.span,
                ));
            }
            let method = declare_method(abc, plan, f)?;
            traits.push(Trait {
                name: abc.pool.qname_in(plan.member_ns, &f.name),
                kind: TraitKind::Method { disp_id: 0, method },
            });
        }
        let iinit = abc.methods.len() as u32;
        abc.methods.push(MethodInfo::default());
        let cinit = emit_empty_initializer(abc, plan.captured, decl.span)?;
        return Ok(ClassEmission {
            iinit,
            cinit,
            instance_traits: traits,
        });
    }

    let mut traits = Vec::new();
    for f in &methods {
        let method = declare_method(abc, plan, f)?;
        let Some(body) = &f.body else {
            return Err(Diagnostic::parse(
                format!("method `{}` has no body", f.name),
                f.span,
            ));
        };
        let code = compile_method_body(abc, plan, f, body, false)?;
        push_body(abc, method, code, plan.captured + 1, f.span)?;
        traits.push(Trait {
            name: abc.pool.qname_in(plan.member_ns, &f.name),
            kind: TraitKind::Method { disp_id: 0, method },
        });
    }

    let iinit = match constructor {
        Some(ctor) => {
            let method = declare_method(abc, plan, ctor)?;
            if !matches!(ctor.sig.return_type, TypeRef::Any) {
                return Err(Diagnostic::parse(
                    "a constructor may not declare a return type",
                    ctor.span,
                ));
            }
            let Some(body) = &ctor.body else {
                return Err(Diagnostic::parse(
                    format!("constructor `{}` has no body", ctor.name),
                    ctor.span,
                ));
            };
            let code = compile_method_body(abc, plan, ctor, body, true)?;
            push_body(abc, method, code, plan.captured + 1, ctor.span)?;
            method
        }
        None => {
            // A class with no declared constructor still needs one: it is what
            // chains to the base class.
            let method = abc.methods.len() as u32;
            abc.methods.push(MethodInfo::default());
            let mut code = CodeBuilder::new();
            code.emit(Op::GetLocal0)
                .emit(Op::PushScope)
                .emit(Op::GetLocal0)
                .emit(Op::ConstructSuper(0))
                .emit(Op::ReturnVoid);
            push_body(abc, method, code, plan.captured + 1, decl.span)?;
            method
        }
    };

    let cinit = emit_empty_initializer(abc, plan.captured, decl.span)?;
    Ok(ClassEmission {
        iinit,
        cinit,
        instance_traits: traits,
    })
}

/// Reserve a `method_info` for `f`, resolving its parameter and return types.
fn declare_method(abc: &mut AbcFile, plan: &ClassPlan<'_>, f: &FunctionDecl) -> Result<u32> {
    let mut param_types = Vec::new();
    for p in &f.sig.params {
        if p.is_rest {
            return Err(Diagnostic::unsupported(
                "rest parameters need the NEED_REST flag and an argument array, which this \
                 phase does not emit",
                p.span,
            ));
        }
        if p.default.is_some() {
            return Err(Diagnostic::unsupported(
                "default parameter values need the method_info optional-value table, which \
                 this phase does not emit",
                p.span,
            ));
        }
        param_types.push(type_multiname(&mut abc.pool, plan, &p.type_ref)?);
    }
    let return_type = type_multiname(&mut abc.pool, plan, &f.sig.return_type)?;
    let index = abc.methods.len() as u32;
    // `method_info.name` is informational — the AVM does not use it — and the
    // shipping reference file leaves it empty too.
    abc.methods.push(MethodInfo {
        param_types,
        return_type,
        name: 0,
        flags: 0,
    });
    Ok(index)
}

fn type_multiname(pool: &mut ConstantPool, plan: &ClassPlan<'_>, ty: &TypeRef) -> Result<u32> {
    Ok(match ty {
        TypeRef::Any => 0,
        TypeRef::Void => pool.qname(NsKind::PackageNamespace, "", "void"),
        TypeRef::Named(name) => {
            let qualified = plan.resolver_resolve(name)?;
            let (pkg, simple) = split_qualified(&qualified);
            pool.qname(NsKind::PackageNamespace, pkg, simple)
        }
    })
}

impl ClassPlan<'_> {
    /// Type names inside a class body resolve exactly as the class header's did.
    fn resolver_resolve(&self, name: &DottedName) -> Result<String> {
        self.resolver.resolve(name)
    }
}

fn emit_empty_initializer(abc: &mut AbcFile, captured: u32, span: Span) -> Result<u32> {
    let method = abc.methods.len() as u32;
    abc.methods.push(MethodInfo::default());
    let mut code = CodeBuilder::new();
    code.emit(Op::ReturnVoid);
    push_body(abc, method, code, captured, span)?;
    Ok(method)
}

fn push_body(
    abc: &mut AbcFile,
    method: u32,
    code: CodeBuilder,
    init_scope_depth: u32,
    span: Span,
) -> Result<()> {
    let min_locals = 1 + abc.methods[method as usize].param_types.len() as u32;
    let stats = code
        .analyze(init_scope_depth, min_locals)
        .map_err(|e| Diagnostic::codegen(e, span))?;
    abc.bodies.push(MethodBody {
        method,
        max_stack: stats.max_stack,
        local_count: stats.local_count,
        init_scope_depth,
        max_scope_depth: stats.max_scope_depth,
        code: code.assemble().map_err(|e| Diagnostic::codegen(e, span))?,
        traits: Vec::new(),
    });
    Ok(())
}

/// The one script initialiser: run on the global object, build each class from
/// its ancestor chain, and publish it as a property of the global object.
fn script_init_code(plans: &[ClassPlan<'_>]) -> CodeBuilder {
    let mut code = CodeBuilder::new();
    code.emit(Op::GetLocal0).emit(Op::PushScope);
    for (index, plan) in plans.iter().enumerate() {
        code.emit(Op::GetScopeObject(0));
        // One scope per ancestor, `Object` first. `WeaponCND.swf` does the same
        // for `MovieClip`, which is what makes its classes capture depth 9.
        for &ancestor in &plan.chain {
            code.emit(Op::GetLex(ancestor)).emit(Op::PushScope);
        }
        // An interface has no base class value; `newclass` takes null.
        if plan.super_name == 0 {
            code.emit(Op::PushNull);
        } else {
            code.emit(Op::GetLex(plan.super_name));
        }
        code.emit(Op::NewClass(index as u32));
        for _ in 0..plan.chain.len() {
            code.emit(Op::PopScope);
        }
        code.emit(Op::InitProperty(plan.name));
    }
    code.emit(Op::ReturnVoid);
    code
}

// ------------------------------------------------------------------ method bodies

struct BodyGen<'a> {
    pool: &'a mut ConstantPool,
    members: &'a HashSet<String>,
    /// Public namespace — where an unqualified property reference is looked up.
    public_ns: u32,
    code: CodeBuilder,
    locals: HashMap<String, u32>,
    next_local: u32,
    /// (break target, continue target) for each enclosing loop.
    loops: Vec<(crate::abc::code::Label, crate::abc::code::Label)>,
}

fn compile_method_body(
    abc: &mut AbcFile,
    plan: &ClassPlan<'_>,
    f: &FunctionDecl,
    body: &Block,
    is_constructor: bool,
) -> Result<CodeBuilder> {
    let public_ns = abc.pool.namespace(NsKind::PackageNamespace, "");
    let mut builder = BodyGen {
        pool: &mut abc.pool,
        members: &plan.members,
        public_ns,
        code: CodeBuilder::new(),
        locals: HashMap::new(),
        next_local: 1,
        loops: Vec::new(),
    };
    for p in &f.sig.params {
        let slot = builder.next_local;
        builder.next_local += 1;
        builder.locals.insert(p.name.clone(), slot);
    }

    // Every method body establishes `this` as its scope first; a constructor
    // then chains to the base class before anything may touch `this`.
    builder.code.emit(Op::GetLocal0).emit(Op::PushScope);

    let mut statements = &body.statements[..];
    if is_constructor {
        // An explicit leading `super(...)` is the constructor's own chain call;
        // otherwise AS3 inserts a zero-argument one.
        let explicit = statements.first().and_then(|s| match s {
            Stmt::Expr(Expr::Call { callee, args, .. }) if matches!(**callee, Expr::Super(_)) => {
                Some(args)
            }
            _ => None,
        });
        builder.code.emit(Op::GetLocal0);
        match explicit {
            Some(args) => {
                for a in args {
                    builder.expr_value(a)?;
                }
                builder.code.emit(Op::ConstructSuper(args.len() as u32));
                statements = &statements[1..];
            }
            None => {
                builder.code.emit(Op::ConstructSuper(0));
            }
        }
    }

    for stmt in statements {
        builder.statement(stmt)?;
    }
    if !ends_in_return(statements) {
        builder.code.emit(Op::ReturnVoid);
    }
    Ok(builder.code)
}

fn ends_in_return(statements: &[Stmt]) -> bool {
    matches!(statements.last(), Some(Stmt::Return(_)))
}

impl BodyGen<'_> {
    /// A property named without a namespace qualifier is looked up in the
    /// public namespace. swftools instead emits a namespace-set multiname and
    /// falls back to late binding; a QName is the precise form and is all
    /// `public`-only members need.
    fn prop(&mut self, name: &str) -> u32 {
        self.pool.qname_in(self.public_ns, name)
    }

    /// The `MultinameL` an indexed access uses. The namespace set holds only the
    /// public namespace, where array elements and dynamic properties live. ASC
    /// emits every open namespace (13 in `WeaponCND.swf`), but this compiler emits
    /// no private, protected or `AS3`-namespaced members, so those are unused.
    fn indexed(&mut self) -> u32 {
        let set = self.pool.ns_set(&[self.public_ns]);
        self.pool.multiname_l(set)
    }

    fn statement(&mut self, stmt: &Stmt) -> Result<()> {
        match stmt {
            Stmt::Empty => Ok(()),
            Stmt::Block(b) => {
                for s in &b.statements {
                    self.statement(s)?;
                }
                Ok(())
            }
            Stmt::Expr(e) => self.expr_discard(e),
            Stmt::Var(v) => {
                if self.locals.contains_key(&v.name) {
                    return Err(Diagnostic::parse(
                        format!("`{}` is declared twice in the same method", v.name),
                        v.span,
                    ));
                }
                let slot = self.next_local;
                self.next_local += 1;
                match &v.init {
                    Some(init) => {
                        self.expr_value(init)?;
                    }
                    // An uninitialised typed local still has to hold something
                    // the verifier accepts before it is read.
                    None => {
                        self.code.emit(Op::PushNull);
                    }
                }
                self.locals.insert(v.name.clone(), slot);
                self.code.emit(Op::SetLocal(slot));
                Ok(())
            }
            Stmt::Return(None) => {
                self.code.emit(Op::ReturnVoid);
                Ok(())
            }
            Stmt::Return(Some(e)) => {
                self.expr_value(e)?;
                // AVM2 coerces a returned value to the method's declared return
                // type, so no explicit coercion is emitted here.
                self.code.emit(Op::ReturnValue);
                Ok(())
            }
            Stmt::If {
                cond,
                then,
                otherwise,
            } => {
                let else_label = self.code.new_label();
                self.expr_value(cond)?;
                self.code.emit(Op::IfFalse(else_label));
                self.statement(then)?;
                match otherwise {
                    None => {
                        self.code.place(else_label);
                    }
                    Some(alt) => {
                        let end = self.code.new_label();
                        self.code.emit(Op::Jump(end));
                        self.code.place(else_label);
                        self.statement(alt)?;
                        self.code.place(end);
                    }
                }
                Ok(())
            }
            Stmt::While { cond, body } => {
                let top = self.code.new_label();
                let end = self.code.new_label();
                self.code.place(top);
                self.expr_value(cond)?;
                self.code.emit(Op::IfFalse(end));
                self.loops.push((end, top));
                self.statement(body)?;
                self.loops.pop();
                self.code.emit(Op::Jump(top));
                self.code.place(end);
                Ok(())
            }
            Stmt::Break(None) => match self.loops.last() {
                Some(&(brk, _)) => {
                    self.code.emit(Op::Jump(brk));
                    Ok(())
                }
                None => Err(Diagnostic::parse("`break` outside a loop", Span::default())),
            },
            Stmt::Continue(None) => match self.loops.last() {
                Some(&(_, cont)) => {
                    self.code.emit(Op::Jump(cont));
                    Ok(())
                }
                None => Err(Diagnostic::parse(
                    "`continue` outside a loop",
                    Span::default(),
                )),
            },
            Stmt::Break(Some(_)) | Stmt::Continue(Some(_)) => Err(Diagnostic::unsupported(
                "labelled `break`/`continue` is not lowered in this phase",
                Span::default(),
            )),
            Stmt::Unsupported { what, span } => Err(Diagnostic::unsupported(
                format!("`{what}` statements are not lowered in this phase"),
                *span,
            )),
        }
    }

    /// Compile an expression used as a statement: nothing may be left behind.
    fn expr_discard(&mut self, e: &Expr) -> Result<()> {
        match e {
            Expr::Call { callee, args, span } => {
                let name = self.call_target(callee, *span)?;
                for a in args {
                    self.expr_value(a)?;
                }
                self.code.emit(Op::CallPropVoid {
                    name,
                    arg_count: args.len() as u32,
                });
                Ok(())
            }
            Expr::Assign {
                target,
                op,
                value,
                span,
            } => self.assign(target, *op, value, *span),
            other => {
                self.expr_value(other)?;
                self.code.emit(Op::Pop);
                Ok(())
            }
        }
    }

    /// Push the receiver of a call and return the multiname to invoke on it.
    fn call_target(&mut self, callee: &Expr, span: Span) -> Result<u32> {
        match callee {
            Expr::Member { object, name, .. } => {
                self.expr_value(object)?;
                Ok(self.prop(name))
            }
            Expr::Ident(name, _) => {
                let mn = self.prop(name);
                if self.locals.contains_key(name) {
                    return Err(Diagnostic::unsupported(
                        format!("calling the local variable `{name}` as a function is not lowered"),
                        span,
                    ));
                }
                if self.members.contains(name) {
                    self.code.emit(Op::GetLocal0);
                } else {
                    // Not a member of this class: leave the lookup to the scope
                    // chain, which is what an unqualified call means.
                    self.code.emit(Op::FindPropStrict(mn));
                }
                Ok(mn)
            }
            Expr::Super(_) => Err(Diagnostic::unsupported(
                "`super.method()` needs callsuper, which this phase does not emit",
                span,
            )),
            _ => Err(Diagnostic::unsupported(
                "only `object.method(...)` and `name(...)` calls are lowered in this phase",
                span,
            )),
        }
    }

    fn assign(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr, span: Span) -> Result<()> {
        if op.is_some() {
            return Err(Diagnostic::unsupported(
                "compound assignment (and `++`/`--`) is not lowered in this phase",
                span,
            ));
        }
        match target {
            Expr::Ident(name, _) if self.locals.contains_key(name) => {
                let slot = self.locals[name];
                self.expr_value(value)?;
                self.code.emit(Op::SetLocal(slot));
                Ok(())
            }
            Expr::Ident(name, _) if self.members.contains(name) => {
                let mn = self.prop(name);
                self.code.emit(Op::GetLocal0);
                self.expr_value(value)?;
                self.code.emit(Op::SetProperty(mn));
                Ok(())
            }
            Expr::Member { object, name, .. } => {
                let mn = self.prop(name);
                self.expr_value(object)?;
                self.expr_value(value)?;
                self.code.emit(Op::SetProperty(mn));
                Ok(())
            }
            Expr::Index { object, index, .. } => {
                let mn = self.indexed();
                self.expr_value(object)?;
                self.expr_value(index)?;
                self.expr_value(value)?;
                self.code.emit(Op::SetPropertyIndexed(mn));
                Ok(())
            }
            _ => Err(Diagnostic::unsupported(
                "only assignment to a local, a field of `this`, `object.property` or \
                 `object[index]` is lowered in this phase",
                span,
            )),
        }
    }

    /// Compile an expression that leaves exactly one value on the stack.
    fn expr_value(&mut self, e: &Expr) -> Result<()> {
        match e {
            Expr::Null(_) => {
                self.code.emit(Op::PushNull);
            }
            Expr::Bool(true, _) => {
                self.code.emit(Op::PushTrue);
            }
            Expr::Bool(false, _) => {
                self.code.emit(Op::PushFalse);
            }
            Expr::Int(v, _) => {
                // A byte-sized literal has a one-operand form and needs no pool
                // entry at all.
                if let Ok(b) = i8::try_from(*v) {
                    self.code.emit(Op::PushByte(b));
                } else if let Ok(i) = i32::try_from(*v) {
                    let idx = self.pool.int(i);
                    self.code.emit(Op::PushInt(idx));
                } else {
                    let idx = self.pool.double(*v as f64);
                    self.code.emit(Op::PushDouble(idx));
                }
            }
            Expr::Number(v, _) => {
                let idx = self.pool.double(*v);
                self.code.emit(Op::PushDouble(idx));
            }
            Expr::Str(s, _) => {
                let idx = self.pool.string(s);
                self.code.emit(Op::PushString(idx));
            }
            Expr::This(_) => {
                self.code.emit(Op::GetLocal0);
            }
            Expr::Ident(name, span) => {
                if let Some(&slot) = self.locals.get(name) {
                    self.code.emit(match slot {
                        1 => Op::GetLocal1,
                        2 => Op::GetLocal2,
                        3 => Op::GetLocal3,
                        n => Op::GetLocal(n),
                    });
                } else if self.members.contains(name) {
                    let mn = self.prop(name);
                    self.code.emit(Op::GetLocal0).emit(Op::GetProperty(mn));
                } else {
                    let mn = self.prop(name);
                    // Unqualified and not ours: `getlex` is exactly
                    // findpropstrict+getproperty, which is how a class or
                    // package-level name resolves.
                    let _ = span;
                    self.code.emit(Op::GetLex(mn));
                }
            }
            Expr::Member { object, name, .. } => {
                let mn = self.prop(name);
                self.expr_value(object)?;
                self.code.emit(Op::GetProperty(mn));
            }
            Expr::Call { callee, args, span } => {
                let name = self.call_target(callee, *span)?;
                for a in args {
                    self.expr_value(a)?;
                }
                self.code.emit(Op::CallProperty {
                    name,
                    arg_count: args.len() as u32,
                });
            }
            Expr::Unary { op, operand, span } => {
                self.expr_value(operand)?;
                match op {
                    UnOp::Not => {
                        self.code.emit(Op::Not);
                    }
                    UnOp::Neg => {
                        self.code.emit(Op::Negate);
                    }
                    UnOp::Plus => {}
                    other => {
                        return Err(Diagnostic::unsupported(
                            format!("unary `{other:?}` is not lowered in this phase"),
                            *span,
                        ));
                    }
                }
            }
            Expr::Binary { op, lhs, rhs, span } => self.binary(*op, lhs, rhs, *span)?,
            Expr::Assign { span, .. } => {
                return Err(Diagnostic::unsupported(
                    "an assignment used as a value is not lowered in this phase; \
                     put it on its own line",
                    *span,
                ));
            }
            Expr::Super(span) => {
                return Err(Diagnostic::unsupported(
                    "`super` is only lowered as a constructor's first statement in this phase",
                    *span,
                ));
            }
            Expr::New { span, .. } => {
                return Err(Diagnostic::unsupported(
                    "`new` needs constructprop, which this phase does not emit",
                    *span,
                ));
            }
            Expr::Index { object, index, .. } => {
                let mn = self.indexed();
                self.expr_value(object)?;
                self.expr_value(index)?;
                self.code.emit(Op::GetPropertyIndexed(mn));
            }
            Expr::ArrayLit { span, .. } => {
                return Err(Diagnostic::unsupported(
                    "array literals need newarray, which this phase does not emit",
                    *span,
                ));
            }
            Expr::Conditional { span, .. } => {
                return Err(Diagnostic::unsupported(
                    "the conditional operator is not lowered in this phase; use `if`",
                    *span,
                ));
            }
        }
        Ok(())
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) -> Result<()> {
        // `&&` and `||` must not evaluate their right operand unconditionally,
        // so they branch rather than reducing to an opcode.
        if matches!(op, BinOp::And | BinOp::Or) {
            let end = self.code.new_label();
            self.expr_value(lhs)?;
            self.code.emit(Op::Dup);
            self.code.emit(if op == BinOp::And {
                Op::IfFalse(end)
            } else {
                Op::IfTrue(end)
            });
            self.code.emit(Op::Pop);
            self.expr_value(rhs)?;
            self.code.place(end);
            return Ok(());
        }

        self.expr_value(lhs)?;
        self.expr_value(rhs)?;
        let simple = match op {
            BinOp::Add => Some(Op::Add),
            BinOp::Sub => Some(Op::Subtract),
            BinOp::Mul => Some(Op::Multiply),
            BinOp::Div => Some(Op::Divide),
            BinOp::Mod => Some(Op::Modulo),
            BinOp::Eq => Some(Op::Equals),
            BinOp::StrictEq => Some(Op::StrictEquals),
            BinOp::Lt => Some(Op::LessThan),
            BinOp::Le => Some(Op::LessEquals),
            BinOp::Gt => Some(Op::GreaterThan),
            BinOp::Ge => Some(Op::GreaterEquals),
            _ => None,
        };
        if let Some(o) = simple {
            self.code.emit(o);
            return Ok(());
        }
        match op {
            // AVM2 has no `notEquals`; negating the equality is what a compiler
            // emits when the result is a value rather than a branch.
            BinOp::Ne => {
                self.code.emit(Op::Equals).emit(Op::Not);
                Ok(())
            }
            BinOp::StrictNe => {
                self.code.emit(Op::StrictEquals).emit(Op::Not);
                Ok(())
            }
            other => Err(Diagnostic::unsupported(
                format!("the `{other:?}` operator is not lowered in this phase"),
                span,
            )),
        }
    }
}
