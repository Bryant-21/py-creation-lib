//! codegen — derive a `PexFilePayload` from a type-checked Papyrus AST.
//!
//! The writer (`pex_writer::write_pex_bytes`) is already byte-faithful, so
//! codegen's only job is to produce the correct payload.
//!
//! ## Determinism note
//! PapyrusCompiler is NOT byte-deterministic: the user-flag name block in the
//! string table AND the function order within a state are emitted in per-process
//! randomized .NET `Hashtable` order. Our codegen is fully deterministic. Parity
//! is therefore checked by ORDER-CANONICALIZED SEMANTIC equivalence (sorted
//! string table + user_flags + per-state functions; resolved instruction/local/
//! debug trees), never raw byte-equality — see the golden tests.

use crate::ast::*;
use crate::parser::ParsedGroup;
use crate::pex::{
    PEX_MAGIC, PexDebugFunctionPayload, PexDebugInfoPayload, PexDebugPropertyGroupPayload,
    PexDebugStructOrderPayload, PexFilePayload, PexFunctionPayload, PexInstructionPayload,
    PexLocalPayload, PexObjectPayload, PexParamPayload, PexPropertyPayload, PexStatePayload,
    PexStructMemberPayload, PexStructPayload, PexUserFlagPayload, PexValuePayload,
    PexVariablePayload,
};
use crate::profile::GameProfile;
use crate::source_resolver::SourceResolver;
use crate::typeck::{NodeId, PapyrusType, TypeckResult};
use std::collections::{HashMap, HashSet};

// --- opcodes ------------------------------------------------------------------
const OP_IADD: u8 = 0x01;
const OP_FADD: u8 = 0x02;
const OP_ISUB: u8 = 0x03;
const OP_FSUB: u8 = 0x04;
const OP_IMUL: u8 = 0x05;
const OP_FMUL: u8 = 0x06;
const OP_IDIV: u8 = 0x07;
const OP_FDIV: u8 = 0x08;
const OP_IMOD: u8 = 0x09;
const OP_NOT: u8 = 0x0A;
const OP_INEG: u8 = 0x0B;
const OP_FNEG: u8 = 0x0C;
const OP_ASSIGN: u8 = 0x0D;
const OP_CAST: u8 = 0x0E;
const OP_CMP_EQ: u8 = 0x0F;
const OP_CMP_LT: u8 = 0x10;
const OP_CMP_LE: u8 = 0x11;
const OP_CMP_GT: u8 = 0x12;
const OP_CMP_GE: u8 = 0x13;
const OP_JMP: u8 = 0x14;
const OP_JMPT: u8 = 0x15;
const OP_JMPF: u8 = 0x16;
const OP_CALLMETHOD: u8 = 0x17;
const OP_CALLPARENT: u8 = 0x18;
const OP_CALLSTATIC: u8 = 0x19;
const OP_RETURN: u8 = 0x1A;
const OP_STRCAT: u8 = 0x1B;
/// Temporary dest name for a deferred argument cast in `build_call`, back-patched
/// with the real `::tempN` once all argument bases are numbered.
const CAST_PLACEHOLDER: &str = "::__cast_ph__";
const OP_PROPGET: u8 = 0x1C;
const OP_PROPSET: u8 = 0x1D;
const OP_ARRAYCREATE: u8 = 0x1E;
const OP_ARRAYLENGTH: u8 = 0x1F;
const OP_ARRAYGETELEMENT: u8 = 0x20;
const OP_ARRAYSETELEMENT: u8 = 0x21;
const OP_STRUCTCREATE: u8 = 0x25;
const OP_STRUCTGET: u8 = 0x26;
const OP_STRUCTSET: u8 = 0x27;
const OP_ARRAYFINDELEMENT: u8 = 0x22;
const OP_ARRAYRFINDELEMENT: u8 = 0x23;
const OP_ARRAYFINDSTRUCT: u8 = 0x28;
const OP_ARRAYRFINDSTRUCT: u8 = 0x29;
const OP_ARRAYADD: u8 = 0x2A;
const OP_ARRAYINSERT: u8 = 0x2B;
const OP_ARRAYREMOVELAST: u8 = 0x2C;
const OP_ARRAYREMOVE: u8 = 0x2D;
const OP_ARRAYCLEAR: u8 = 0x2E;

// --- value-type tags (mirror pex::read_value) ---------------------------------
const VT_NONE: u8 = 0;
const VT_IDENT: u8 = 1;
const VT_STRING: u8 = 2;
const VT_INT: u8 = 3;
const VT_FLOAT: u8 = 4;
const VT_BOOL: u8 = 5;

pub fn compile(
    ast: &ScriptNode,
    tc: &TypeckResult,
    resolver: &SourceResolver,
    profile: GameProfile,
    _flags: Option<&str>,
    script_docstring: &str,
    property_groups: &[ParsedGroup],
    struct_names: &[String],
    source_script_name: Option<&str>,
) -> PexFilePayload {
    Codegen::new(
        ast,
        tc,
        resolver,
        profile,
        script_docstring,
        property_groups,
        struct_names,
        source_script_name,
    )
    .run()
}

struct Codegen<'a> {
    ast: &'a ScriptNode,
    tc: &'a TypeckResult,
    resolver: &'a SourceResolver,
    profile: GameProfile,
    /// Expr address → NodeId, from the shared canonical walk (matches typeck).
    expr_ids: HashMap<usize, NodeId>,
    /// `::tempN` counter — script-global, monotonic, never reset (empirically
    /// confirmed: a 2nd function continues the numbering).
    temp_counter: u32,
    /// Per-type FIFO free-list of temps available for reuse, keyed by the
    /// lowercased type string (PCompiler `pUnusedTempVarsByType`). A temp of an
    /// exact type is reused before a fresh `::tempN` is minted. Cleared at the
    /// start of each function/event. Function-global across nested blocks.
    unused_temps: HashMap<String, Vec<String>>,
    /// Auto-property name (lowercased) → backing variable `::<Name>_var`. In-
    /// script reads/writes of an auto property hit the backing var directly,
    /// never PROPGET/PROPSET (empirically confirmed).
    prop_vars: HashMap<String, String>,
    /// Whether the per-function shared `::nonevar` (None) sentinel for discarded
    /// void call results has been allocated yet.
    nonevar_used: bool,
    /// Script-level names (member variables + properties) → type. Used to type
    /// `NameExpr`s the type checker leaves unresolved (e.g. property handler
    /// bodies, which it does not scope).
    members: HashMap<String, PapyrusType>,
    /// Current callable's params + declared locals → type (reset per callable).
    scope: HashMap<String, PapyrusType>,
    // ---- per-function scratch (reset in `lower_function`) ----
    instrs: Vec<PexInstructionPayload>,
    debug_lines: Vec<u16>,
    /// Block-scope stack for locals ordering. Each block contributes its own
    /// temps first, then its statements' declared locals / nested blocks, in
    /// source order — PCompiler's per-block prepend (§4). Flattened depth-first
    /// at function exit into the `.localTable`.
    block_stack: Vec<BlockFrame>,
    /// Label id → instruction index it resolves to (the index of the next
    /// instruction emitted after the label is placed).
    labels: HashMap<u32, usize>,
    /// (instruction index of a jump, target label) — relative offsets are
    /// back-patched after the function body is fully laid out.
    pending_jumps: Vec<(usize, u32)>,
    label_counter: u32,
    /// Script-level doc comment (object docstring); the parser threads it here
    /// because `ScriptNode` (ast.rs) carries no docstring field.
    script_docstring: &'a str,
    /// Shadow-mangling (PapyrusGen.MangleFunctionVariables). `iCurMangleSuffix`
    /// is script-global, never reset — like the temp counter.
    mangle_counter: u32,
    /// Lowercased local names already declared in the CURRENT function. The first
    /// occurrence keeps its name; a later re-declaration is shadow-mangled.
    declared_locals: HashSet<String>,
    /// Active lowercased-name -> emitted-name overrides for shadowed locals,
    /// scoped to the block that declared them (restored on block exit).
    local_aliases: HashMap<String, String>,
    /// Lowercased local/param name -> its DECLARED casing. References emit the
    /// declaration casing (stock interns one casing per identifier), not the
    /// source-reference-site casing.
    local_names: HashMap<String, String>,
    /// Script-global lowercased identifier -> FIRST-SEEN casing. Stock interns
    /// one casing per identifier across the whole script (e.g. `Debug.Trace`
    /// then `debug.trace` both emit the first occurrence's casing). NOT reset
    /// per function.
    ident_casing: HashMap<String, String>,
    /// `Group` declarations (source order) for the FO4 debug property-group
    /// table; threaded from the parser because `ScriptNode` carries no groups.
    property_groups: &'a [ParsedGroup],
    /// Same-script struct names (parser discards struct bodies); used to qualify
    /// struct-typed fields as `script#struct`.
    struct_names: &'a [String],
    source_script_name: Option<&'a str>,
}

impl<'a> Codegen<'a> {
    fn new(
        ast: &'a ScriptNode,
        tc: &'a TypeckResult,
        resolver: &'a SourceResolver,
        profile: GameProfile,
        script_docstring: &'a str,
        property_groups: &'a [ParsedGroup],
        struct_names: &'a [String],
        source_script_name: Option<&'a str>,
    ) -> Self {
        let mut expr_ids = HashMap::new();
        crate::compiler::walk_preorder(ast, |id, node| {
            if let crate::compiler::WalkNode::Expr(e) = node {
                expr_ids.insert(e as *const Expr as usize, id);
            }
        });
        let mut prop_vars = HashMap::new();
        let mut members = HashMap::new();
        for p in &ast.properties {
            members.insert(p.name.to_lowercase(), parse_ty(&p.ty));
            if p.flags.iter().any(|f| f.eq_ignore_ascii_case("Auto")) {
                prop_vars.insert(p.name.to_lowercase(), format!("::{}_var", p.name));
            }
        }
        // Accessing an INHERITED auto-property reads/writes the declaring
        // ancestor's backing variable `::Name_var` directly. Own properties take
        // precedence (or_insert), so an override keeps this script's mapping.
        if let Some(parent) = &ast.parent {
            for ancestor in resolver.get_hierarchy(parent) {
                if let Some(anc) = resolver.parsed(&ancestor) {
                    for p in &anc.properties {
                        // Record the inherited type so reads don't spuriously cast
                        // (own members already inserted, so they win).
                        members
                            .entry(p.name.to_lowercase())
                            .or_insert_with(|| parse_ty(&p.ty));
                        if p.flags.iter().any(|f| f.eq_ignore_ascii_case("Auto")) {
                            prop_vars
                                .entry(p.name.to_lowercase())
                                .or_insert_with(|| format!("::{}_var", p.name));
                        }
                    }
                }
            }
        }
        for v in &ast.variables {
            members.insert(v.name.to_lowercase(), parse_ty(&v.ty));
        }
        Self {
            ast,
            tc,
            resolver,
            profile,
            expr_ids,
            temp_counter: 0,
            unused_temps: HashMap::new(),
            prop_vars,
            nonevar_used: false,
            members,
            scope: HashMap::new(),
            instrs: Vec::new(),
            debug_lines: Vec::new(),
            block_stack: Vec::new(),
            labels: HashMap::new(),
            pending_jumps: Vec::new(),
            label_counter: 0,
            script_docstring,
            property_groups,
            struct_names,
            source_script_name,
            mangle_counter: 0,
            declared_locals: HashSet::new(),
            local_aliases: HashMap::new(),
            local_names: HashMap::new(),
            ident_casing: HashMap::new(),
        }
    }

    fn emitted_script_name(&self) -> String {
        self.source_script_name
            .unwrap_or(&self.ast.name)
            .to_string()
    }

    /// Return `name` in its script-global FIRST-SEEN casing, locking this casing
    /// if it is the first occurrence (stock interns one casing per identifier).
    fn intern_ident(&mut self, name: &str) -> String {
        self.ident_casing
            .entry(name.to_lowercase())
            .or_insert_with(|| name.to_string())
            .clone()
    }

    fn run(mut self) -> PexFilePayload {
        let parent = self.parent_name();

        // Every function/event lives in the unnamed default state for Batch 1.
        // Iterate the ORIGINAL AST nodes (not a clone) so expression addresses
        // match the NodeId map built from `self.ast` — cloning would break the
        // `type_of_expr` lookups.
        let ast = self.ast;
        let mut debug_functions = Vec::new();

        // The script-global ::tempN counter advances in pure SOURCE ORDER across
        // every callable, regardless of which state owns it (the type-walker does
        // not group default-state functions ahead of named-state ones). Collect
        // all callables tagged with their state, lower them in line order so the
        // temp numbering matches stock, then route each payload back to its
        // state. Within-state function order is canonicalized away. The exe
        // lowercases a non-auto state's name but keeps the auto state's source
        // casing (which also lands in the object's auto_state header field).
        let mut auto_state = String::new();
        let mut state_names: Vec<String> = vec![String::new()];
        let mut work: Vec<(String, FnOrEvent)> = source_ordered(&ast.functions, &ast.events)
            .into_iter()
            .map(|it| (String::new(), it))
            .collect();
        for st in &ast.states {
            let sname = if st.is_auto {
                st.name.clone()
            } else {
                st.name.to_lowercase()
            };
            if st.is_auto {
                auto_state = sname.clone();
            }
            if !state_names.contains(&sname) {
                state_names.push(sname.clone());
            }
            for it in source_ordered(&st.functions, &st.events) {
                work.push((sname.clone(), it));
            }
        }
        work.sort_by_key(|(_, it)| it.line());
        // Member-variable names are interned (stock writes the variable table
        // before function bodies) so their DECLARED casing wins over a later
        // reference-site casing.
        for v in &ast.variables {
            self.intern_ident(&v.name);
        }
        let mut state_funcs: HashMap<String, Vec<PexFunctionPayload>> = HashMap::new();
        for (sname, it) in &work {
            let (payload, dbg) = match it {
                FnOrEvent::Fn(f) => self.func_from_def(f, &f.name, &f.name, sname, 0),
                FnOrEvent::Event(e) => self.lower_event(e, sname),
            };
            state_funcs.entry(sname.clone()).or_default().push(payload);
            debug_functions.push(dbg);
        }
        let states: Vec<PexStatePayload> = state_names
            .into_iter()
            .map(|name| {
                let functions = state_funcs.remove(&name).unwrap_or_default();
                PexStatePayload { name, functions }
            })
            .collect();

        let (properties, variables) = self.build_props_and_vars(&mut debug_functions);

        let structs: Vec<PexStructPayload> =
            self.ast
                .structs
                .iter()
                .map(|s| PexStructPayload {
                    name: s.name.clone(),
                    members: s
                        .members
                        .iter()
                        .map(|m| PexStructMemberPayload {
                            name: m.name.clone(),
                            ty: type_string_for_decl(&m.ty),
                            user_flags: self.user_flags_mask(&m.flags),
                            data: m.value.as_ref().and_then(const_value).unwrap_or(
                                PexValuePayload {
                                    value_type: VT_NONE,
                                    data: serde_json::Value::Null,
                                },
                            ),
                            is_const: has_flag(&m.flags, "Const"),
                            docstring: String::new(),
                        })
                        .collect(),
                })
                .collect();

        let object_name = self.emitted_script_name();
        let mut object = PexObjectPayload {
            name: object_name.clone(),
            parent,
            docstring: self.script_docstring.to_string(),
            is_const: has_flag(&self.ast.flags, "Const"),
            auto_state,
            structs,
            user_flags: self.user_flags_mask(&self.ast.flags),
            variables,
            guards: Vec::new(),
            properties,
            states,
        };
        // Struct names come from the parser (it discards struct bodies but keeps
        // the names); fall back to any `ScriptNode.structs` entries too.
        let struct_names: HashSet<String> = self
            .struct_names
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .chain(self.ast.structs.iter().map(|s| s.name.to_ascii_lowercase()))
            .collect();
        apply_type_name_case(&mut object, &struct_names, self.resolver);

        // FO4 debug property groups: each `Group` becomes a named entry, with
        // properties declared outside any group sharing the empty-name group.
        // Groups (including the empty one) are emitted in the order their first
        // member property is declared.
        let mut prop_to_group: HashMap<String, &ParsedGroup> = HashMap::new();
        for g in self.property_groups {
            for n in &g.prop_names {
                prop_to_group.insert(n.to_lowercase(), g);
            }
        }
        let mut order: Vec<String> = Vec::new();
        let mut members: HashMap<String, Vec<String>> = HashMap::new();
        for p in &object.properties {
            let gname = prop_to_group
                .get(&p.name.to_lowercase())
                .map(|g| g.name.clone())
                .unwrap_or_default();
            if !members.contains_key(&gname) {
                order.push(gname.clone());
            }
            members.entry(gname).or_default().push(p.name.clone());
        }
        let property_groups: Vec<PexDebugPropertyGroupPayload> = order
            .into_iter()
            .map(|gname| {
                let meta = self.property_groups.iter().find(|g| g.name == gname);
                PexDebugPropertyGroupPayload {
                    object_name: object.name.clone(),
                    group_name: gname.clone(),
                    docstring: meta.map(|g| g.docstring.clone()).unwrap_or_default(),
                    user_flags: meta.map(|g| self.user_flags_mask(&g.flags)).unwrap_or(0),
                    property_names: members.remove(&gname).unwrap_or_default(),
                }
            })
            .collect();

        // Declaration-order member names per struct (deterministic; the struct
        // member LIST in the table permutes via Hashtable but this debug order
        // is source order).
        let struct_orders: Vec<PexDebugStructOrderPayload> = self
            .ast
            .structs
            .iter()
            .map(|s| PexDebugStructOrderPayload {
                object_name: object_name.clone(),
                struct_name: s.name.clone(),
                member_names: s.members.iter().map(|m| m.name.clone()).collect(),
            })
            .collect();

        // Debug block is emitted only when there is something to describe
        // (an empty object — like the slice — has no debug block).
        let debug_info =
            if debug_functions.is_empty() && property_groups.is_empty() && struct_orders.is_empty()
            {
                None
            } else {
                Some(PexDebugInfoPayload {
                    modification_time: 0, // neutralized; compile_source also zeroes it
                    functions: debug_functions,
                    property_groups,
                    struct_orders,
                })
            };

        let user_flags = default_user_flags(self.profile);
        let string_table = self.build_string_table(&user_flags, &object, debug_info.as_ref());

        PexFilePayload {
            magic: PEX_MAGIC,
            major_version: self.profile.major_version,
            minor_version: self.profile.minor_version,
            game_id: self.profile.game_id,
            compilation_time: 0,
            source_filename: format!("{}.psc", self.ast.name),
            username: String::new(),
            machine_name: String::new(),
            string_table,
            debug_info,
            user_flags,
            objects: vec![object],
        }
    }

    /// FO4 scripts with no explicit `extends` implicitly extend `ScriptObject`.
    fn parent_name(&self) -> String {
        match self.ast.parent.as_deref() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => "ScriptObject".to_string(),
        }
    }

    /// Member variables, then properties. Auto properties emit a property
    /// (flags read|write|autovar = 7) plus a backing variable `::<Name>_var`.
    /// Handler properties emit a property whose flags are read|write driven by
    /// getter/setter presence, with the bodies lowered into `get_<P>`/`set_<P>`
    /// functions (debug type 1/2, named after the property).
    /// User-flag bitmask for every user-flag-table name present in `flags`
    /// (each flag's table index → bit). Used for the object record's script-level
    /// flags (e.g. `Hidden` → bit 0).
    fn user_flags_mask(&self, flags: &[String]) -> u32 {
        default_user_flags(self.profile)
            .iter()
            .filter(|uf| flags.iter().any(|f| f.eq_ignore_ascii_case(&uf.name)))
            .fold(0u32, |bits, uf| bits | (1u32 << uf.index))
    }

    /// User-flag bitmask a property/variable's flags contribute to its backing
    /// variable. Only `Conditional` propagates to the variable (its bit index in
    /// the user-flag table); `Mandatory`/`Hidden`/etc. stay on the property.
    fn var_user_flags(&self, flags: &[String]) -> u32 {
        if !has_flag(flags, "Conditional") {
            return 0;
        }
        self.conditional_bit()
    }

    fn conditional_bit(&self) -> u32 {
        default_user_flags(self.profile)
            .iter()
            .find(|uf| uf.name == "conditional")
            .map(|uf| 1u32 << uf.index)
            .unwrap_or(0)
    }

    /// User-flag bitmask on a property RECORD: all of the property's user flags
    /// (Mandatory/Hidden/etc.) except Conditional, which the exe places on the
    /// backing variable instead.
    fn prop_user_flags(&self, flags: &[String]) -> u32 {
        self.user_flags_mask(flags) & !self.conditional_bit()
    }

    fn build_props_and_vars(
        &mut self,
        debug_functions: &mut Vec<PexDebugFunctionPayload>,
    ) -> (Vec<PexPropertyPayload>, Vec<PexVariablePayload>) {
        let ast = self.ast;
        let mut properties = Vec::new();
        let mut variables = Vec::new();
        for v in &ast.variables {
            variables.push(PexVariablePayload {
                name: v.name.clone(),
                ty: type_string_for_decl(&v.ty),
                user_flags: self.var_user_flags(&v.flags),
                data: v
                    .value
                    .as_ref()
                    .and_then(const_value)
                    .unwrap_or(PexValuePayload {
                        value_type: VT_NONE,
                        data: serde_json::Value::Null,
                    }),
                is_const: has_flag(&v.flags, "Const"),
            });
        }
        for p in &ast.properties {
            let is_auto = p.flags.iter().any(|f| f.eq_ignore_ascii_case("Auto"));
            let ty = type_string_for_decl(&p.ty);
            if is_auto {
                let var_name = format!("::{}_var", p.name);
                properties.push(PexPropertyPayload {
                    name: p.name.clone(),
                    ty: ty.clone(),
                    docstring: p.docstring.clone(),
                    user_flags: self.prop_user_flags(&p.flags),
                    flags: 7,
                    auto_var: var_name.clone(),
                    getter: None,
                    setter: None,
                });
                // The backing variable inherits the property's const-ness, the
                // Conditional user-flag (the exe puts Conditional on the var, not
                // the property record), and the property's default value as its
                // initial data.
                variables.push(PexVariablePayload {
                    name: var_name,
                    ty,
                    user_flags: self.var_user_flags(&p.flags),
                    data: p
                        .default
                        .as_ref()
                        .and_then(const_value)
                        .unwrap_or(PexValuePayload {
                            value_type: VT_NONE,
                            data: serde_json::Value::Null,
                        }),
                    is_const: has_flag(&p.flags, "Const"),
                });
                continue;
            }
            // Handler property: lower getter/setter bodies; flags from presence.
            let mut flags = 0u8;
            let getter = p.getter.as_ref().map(|g| {
                flags |= 1;
                let (payload, dbg) =
                    self.func_from_def(g, &format!("get_{}", p.name), &p.name, "", 1);
                debug_functions.push(dbg);
                payload
            });
            let setter = p.setter.as_ref().map(|s| {
                flags |= 2;
                let (payload, dbg) =
                    self.func_from_def(s, &format!("set_{}", p.name), &p.name, "", 2);
                debug_functions.push(dbg);
                payload
            });
            properties.push(PexPropertyPayload {
                name: p.name.clone(),
                ty,
                docstring: p.docstring.clone(),
                user_flags: self.prop_user_flags(&p.flags),
                flags,
                auto_var: String::new(),
                getter,
                setter,
            });
        }
        (properties, variables)
    }

    // --- function / event lowering --------------------------------------------

    /// Lower a statement body into (locals, instructions, debug line numbers).
    /// Resets all per-function scratch first.
    fn lower_body_to_parts(
        &mut self,
        body: &[Stmt],
        return_type: &str,
        params: &[Parameter],
    ) -> (Vec<PexLocalPayload>, Vec<PexInstructionPayload>, Vec<u16>) {
        self.instrs.clear();
        self.debug_lines.clear();
        self.block_stack.clear();
        self.labels.clear();
        self.pending_jumps.clear();
        self.nonevar_used = false;
        self.scope.clear();
        // The temp free-list is per-function (PCompiler clears it at function
        // end); the ::tempN counter is NOT reset — it runs script-global.
        self.unused_temps.clear();
        // Shadow-mangling: the "already defined" set is per-function, but the
        // mangle SUFFIX counter is script-global (never reset here).
        self.declared_locals.clear();
        self.local_aliases.clear();
        self.local_names.clear();
        for p in params {
            self.scope.insert(p.name.to_lowercase(), parse_ty(&p.ty));
            self.local_names
                .insert(p.name.to_lowercase(), p.name.to_string());
        }

        self.block_stack.push(BlockFrame::default());
        for stmt in body {
            self.lower_stmt(stmt, return_type);
        }
        self.resolve_jumps();
        let locals = self.block_stack.pop().unwrap().flatten();
        (
            locals,
            std::mem::take(&mut self.instrs),
            std::mem::take(&mut self.debug_lines),
        )
    }

    fn dbg_fn(&self, name: &str, state: &str, ty: u8, lines: Vec<u16>) -> PexDebugFunctionPayload {
        PexDebugFunctionPayload {
            object_name: self.emitted_script_name(),
            state_name: state.to_string(),
            function_name: name.to_string(),
            function_type: ty,
            line_numbers: lines,
        }
    }

    fn params_of(f: &FunctionDef) -> Vec<PexParamPayload> {
        f.params
            .iter()
            .map(|p| PexParamPayload {
                name: p.name.clone(),
                ty: type_string_for_decl(&p.ty),
            })
            .collect()
    }

    /// Lower a `FunctionDef` (also used for property getters/setters and named-
    /// state functions). `emit_name` is the `.pex` function name (e.g. `get_P`);
    /// `dbg_name`/`dbg_type` drive the debug entry (property handlers use the
    /// property name with type 1/2).
    fn func_from_def(
        &mut self,
        f: &FunctionDef,
        emit_name: &str,
        dbg_name: &str,
        dbg_state: &str,
        dbg_type: u8,
    ) -> (PexFunctionPayload, PexDebugFunctionPayload) {
        let (locals, instructions, lines) =
            self.lower_body_to_parts(&f.body, &f.return_type, &f.params);
        let payload = PexFunctionPayload {
            name: emit_name.to_string(),
            return_type: type_string_for_decl(&f.return_type),
            docstring: f.docstring.clone(),
            user_flags: 0,
            is_native: f.is_native,
            is_global: f.is_global,
            params: Self::params_of(f),
            locals,
            instructions,
        };
        let dbg = self.dbg_fn(dbg_name, dbg_state, dbg_type, lines);
        (payload, dbg)
    }

    /// Events lower like void, non-global functions (debug type 0).
    fn lower_event(
        &mut self,
        e: &EventDef,
        dbg_state: &str,
    ) -> (PexFunctionPayload, PexDebugFunctionPayload) {
        let (locals, instructions, lines) = self.lower_body_to_parts(&e.body, "None", &e.params);
        let fn_name = remote_event_name(&e.name);
        let payload = PexFunctionPayload {
            name: fn_name.clone(),
            return_type: "None".to_string(),
            docstring: e.docstring.clone(),
            user_flags: 0,
            is_native: e.is_native,
            is_global: false,
            params: e
                .params
                .iter()
                .map(|p| PexParamPayload {
                    name: p.name.clone(),
                    ty: type_string_for_decl(&p.ty),
                })
                .collect(),
            locals,
            instructions,
        };
        let dbg = self.dbg_fn(&fn_name, dbg_state, 0, lines);
        (payload, dbg)
    }

    fn lower_stmt(&mut self, stmt: &Stmt, ret_type: &str) {
        match stmt {
            Stmt::ReturnStmt { value, pos } => {
                let line = pos.line as u16;
                let operand = match value {
                    Some(expr) => {
                        let v = self.lower_expr(expr, line);
                        self.cast_to(v, self.type_of_expr(expr), &parse_ty(ret_type), line)
                    }
                    None => PexValuePayload {
                        value_type: VT_NONE,
                        data: serde_json::Value::Null,
                    },
                };
                self.emit(OP_RETURN, vec![operand], line);
                self.mark_all_temps_unused();
            }
            Stmt::LocalVarStmt {
                name,
                ty,
                value,
                pos,
            } => {
                let line = pos.line as u16;
                self.scope.insert(name.to_lowercase(), parse_ty(ty));
                let emit_name = self.declare_local(name);
                self.block_stack
                    .last_mut()
                    .unwrap()
                    .entries
                    .push(LocalEntry::Decl(PexLocalPayload {
                        name: emit_name.clone(),
                        ty: type_string_for_decl(ty),
                    }));
                if let Some(expr) = value {
                    let target_ty = parse_ty(ty);
                    let v = self.lower_expr_for_target(expr, line, &target_ty);
                    self.emit(OP_ASSIGN, vec![ident(&emit_name), v], line);
                } else {
                    let lty = parse_ty(ty);
                    if let Some(default) = default_local_value(&lty) {
                        self.emit(OP_ASSIGN, vec![ident(&emit_name), default], line);
                    } else if matches!(
                        lty,
                        PapyrusType::Object(_) | PapyrusType::Array(_) | PapyrusType::Struct(_)
                    ) {
                        let none = PexValuePayload {
                            value_type: VT_NONE,
                            data: serde_json::Value::Null,
                        };
                        let v = self.cast_to(none, PapyrusType::None, &lty, line);
                        self.emit(OP_ASSIGN, vec![ident(&emit_name), v], line);
                    }
                }
            }
            Stmt::ExprStmt { expr, pos } => {
                // Evaluate for side effects; the result operand is discarded.
                let _ = self.lower_expr(expr, pos.line as u16);
                self.mark_all_temps_unused();
            }
            Stmt::AssignStmt {
                target,
                op,
                value,
                pos,
            } => {
                let line = pos.line as u16;
                // Compound ops (`+=` …) are a later batch; `=` only here.
                if op != "=" {
                    return;
                }
                match target {
                    Expr::NameExpr { name, .. } => {
                        let target_ty = self.type_of_expr(target);
                        let v = self.lower_expr_for_target(value, line, &target_ty);
                        self.emit(OP_ASSIGN, vec![ident(&self.resolve_name(name)), v], line);
                    }
                    Expr::ArrayAccessExpr { array, index, .. } => {
                        // RHS is materialized into a temp of the element type, then
                        // ARRAYSETELEMENT writes it. Stock numbers that dest temp
                        // BEFORE the value's own (cast) temp — allocate it first.
                        let elem = self.type_of_expr(target);
                        let arr = self.lower_expr(array, line);
                        let idx = self.lower_expr(index, line);
                        let dest = self.alloc_temp(&elem);
                        let v = self.lower_expr(value, line);
                        let v = self.cast_to(v, self.type_of_expr(value), &elem, line);
                        self.emit(OP_ASSIGN, vec![ident(&dest), v], line);
                        self.emit(OP_ARRAYSETELEMENT, vec![arr, idx, ident(&dest)], line);
                    }
                    // `obj.Property = value` on another object -> PROPSET (a bare
                    // `Prop =` self auto-property is the NameExpr path -> backing
                    // var). Stock numbers the object temps first, then the value
                    // dest, but EMITS the value before the object, and ALWAYS
                    // materializes the value into its dest via ASSIGN. We lower the
                    // object first (to number its temps), buffer its instructions,
                    // emit the value, then re-emit the object. Reordering is only
                    // safe when the object produced no jumps/labels (no control
                    // flow) — otherwise the buffered jump indices would shift.
                    Expr::DotExpr { object, member, .. } => {
                        let prop_ty = self.type_of_expr(target);
                        let jumps_before = self.pending_jumps.len();
                        let labels_before = self.labels.len();
                        let instr_start = self.instrs.len();
                        let obj = self.lower_expr(object, line);
                        let reorderable = self.pending_jumps.len() == jumps_before
                            && self.labels.len() == labels_before;
                        let buffered = if reorderable {
                            let i: Vec<_> = self.instrs.drain(instr_start..).collect();
                            let l: Vec<_> = self.debug_lines.drain(instr_start..).collect();
                            Some((i, l))
                        } else {
                            None
                        };
                        let is_struct = self
                            .struct_member_ty(&self.type_of_expr(object).to_string(), member)
                            .is_some();
                        let dest = self.alloc_temp(&prop_ty);
                        let raw = self.lower_expr(value, line);
                        let v = self.cast_to(raw, self.type_of_expr(value), &prop_ty, line);
                        self.emit(OP_ASSIGN, vec![ident(&dest), v], line);
                        if let Some((i, l)) = buffered {
                            self.instrs.extend(i);
                            self.debug_lines.extend(l);
                        }
                        if is_struct {
                            // `structVal.member = value` -> STRUCT_SET [struct, member, value].
                            self.emit(OP_STRUCTSET, vec![obj, ident(member), ident(&dest)], line);
                        } else {
                            self.emit(OP_PROPSET, vec![ident(member), obj, ident(&dest)], line);
                        }
                    }
                    _ => {}
                }
                self.mark_all_temps_unused();
            }
            Stmt::IfStmt {
                condition,
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                let end = self.new_label();
                self.lower_branch(condition, body, end, ret_type);
                for c in elseif_clauses {
                    self.lower_branch(&c.condition, &c.body, end, ret_type);
                }
                self.enter_block();
                for s in else_body {
                    self.lower_stmt(s, ret_type);
                }
                self.exit_block();
                self.place_label(end);
                self.mark_all_temps_unused();
            }
            Stmt::WhileStmt {
                condition, body, ..
            } => {
                let start = self.new_label();
                let end = self.new_label();
                self.place_label(start);
                let line = condition.pos().line as u16;
                let cond = self.lower_expr(condition, line);
                self.emit_jump(OP_JMPF, Some(cond), end, line);
                self.enter_block();
                for s in body {
                    self.lower_stmt(s, ret_type);
                }
                self.exit_block();
                self.emit_jump(OP_JMP, None, start, self.last_line());
                self.place_label(end);
                self.mark_all_temps_unused();
            }
        }
    }

    /// One `if`/`elseif` clause: `cond`; `JMPF cond → elseLabel`; `body`;
    /// `JMP → endLabel`; `elseLabel:`.
    fn lower_branch(&mut self, cond: &Expr, body: &[Stmt], end: u32, ret_type: &str) {
        let line = cond.pos().line as u16;
        let c = self.lower_expr(cond, line);
        let else_label = self.new_label();
        self.emit_jump(OP_JMPF, Some(c), else_label, line);
        self.enter_block();
        for s in body {
            self.lower_stmt(s, ret_type);
        }
        self.exit_block();
        self.emit_jump(OP_JMP, None, end, self.last_line());
        self.place_label(else_label);
    }

    // --- expression lowering --------------------------------------------------

    /// Lower `expr`, emitting any needed instructions, and return the operand
    /// that holds its result (a literal, an identifier, or a temp identifier).
    fn lower_expr(&mut self, expr: &Expr, line: u16) -> PexValuePayload {
        self.lower_expr_with_expected(expr, line, None)
    }

    fn lower_expr_with_expected(
        &mut self,
        expr: &Expr,
        line: u16,
        expected: Option<&PapyrusType>,
    ) -> PexValuePayload {
        match expr {
            Expr::LiteralExpr { value, ty, .. } => literal_value(value, ty),
            Expr::NameExpr { name, .. } => ident(&self.resolve_name(name)),
            Expr::BinaryExpr {
                left, op, right, ..
            } => {
                if op == "&&" || op == "||" {
                    self.lower_logical(op, left, right, line)
                } else if is_comparison(op) {
                    self.lower_comparison(op, left, right, line)
                } else {
                    self.lower_binary(expr, left, op, right, line)
                }
            }
            Expr::UnaryExpr { op, operand, .. } => self.lower_unary(expr, op, operand, line),
            Expr::CastExpr {
                expr: inner,
                target_type,
                ..
            } => {
                let v = self.lower_expr(inner, line);
                self.cast_to(v, self.type_of_expr(inner), &parse_ty(target_type), line)
            }
            Expr::ArrayAccessExpr { array, index, .. } => {
                let arr = self.lower_expr(array, line);
                let idx = self.lower_expr(index, line);
                let elem = self.type_of_expr(expr);
                let dest = self.alloc_temp(&elem);
                self.emit(OP_ARRAYGETELEMENT, vec![ident(&dest), arr, idx], line);
                ident(&dest)
            }
            Expr::NewArrayExpr { size, .. } => {
                let n = self.lower_expr(size, line);
                let dest = self.alloc_temp(&self.type_of_expr(expr));
                self.emit(OP_ARRAYCREATE, vec![ident(&dest), n], line);
                ident(&dest)
            }
            Expr::DotExpr { object, member, .. }
                if member.eq_ignore_ascii_case("Length")
                    && matches!(self.type_of_expr(object), PapyrusType::Array(_)) =>
            {
                let arr = self.lower_expr(object, line);
                let dest = self.alloc_temp(&PapyrusType::Int);
                self.emit(OP_ARRAYLENGTH, vec![ident(&dest), arr], line);
                ident(&dest)
            }
            Expr::DotExpr { object, member, .. } => {
                let obj_ty = self.type_of_expr(object);
                if let Some(mty) = self.struct_member_ty(&obj_ty.to_string(), member) {
                    // `structVal.member` read -> STRUCT_GET [dest, struct, member].
                    let obj = self.lower_expr(object, line);
                    let dest = self.alloc_temp(&mty);
                    self.emit(OP_STRUCTGET, vec![ident(&dest), obj, ident(member)], line);
                    ident(&dest)
                } else {
                    // `obj.Property` read on another object -> PROPGET (self auto-
                    // property reads come through NameExpr -> backing var).
                    let obj = self.lower_expr(object, line);
                    let dest = self.alloc_temp(&self.type_of_expr(expr));
                    self.emit(OP_PROPGET, vec![ident(member), obj, ident(&dest)], line);
                    ident(&dest)
                }
            }
            Expr::CallExpr {
                function,
                args,
                arg_names,
                ..
            } => {
                // `new <Struct>` parses as a synthetic call -> STRUCT_CREATE [dest].
                if let Some(struct_ty) = function.strip_prefix("new ") {
                    let dest = self.alloc_temp(&PapyrusType::Object(struct_ty.to_string()));
                    self.emit(OP_STRUCTCREATE, vec![ident(&dest)], line);
                    ident(&dest)
                } else {
                    self.lower_self_call(function, args, arg_names, line)
                }
            }
            Expr::DotCallExpr {
                object,
                method,
                args,
                ..
            } => self.lower_dot_call(object, method, args, line, expected),
            // Other expression kinds are added in later batches.
            _ => PexValuePayload {
                value_type: VT_NONE,
                data: serde_json::Value::Null,
            },
        }
    }

    fn lower_expr_for_target(
        &mut self,
        expr: &Expr,
        line: u16,
        target: &PapyrusType,
    ) -> PexValuePayload {
        let v = self.lower_expr_with_expected(expr, line, Some(target));
        let from = if self.is_unresolved_static_dot_call(expr) {
            target.clone()
        } else {
            self.type_of_expr(expr)
        };
        self.cast_to(v, from, target, line)
    }

    /// `F(args)` with no receiver: a same-script global → `CALLSTATIC`, otherwise
    /// a method on `self` (same-script or inherited) → `CALLMETHOD … self …`.
    fn lower_self_call(
        &mut self,
        function: &str,
        args: &[Expr],
        arg_names: &[Option<String>],
        line: u16,
    ) -> PexValuePayload {
        let owner = self.ast.name.clone();
        // A bare call not found on self or its hierarchy may be an imported
        // global (e.g. `Import Utility` + `RandomFloat(...)`) — emit it as a
        // static call on the imported script.
        if self.resolve_callee(&owner, function).is_none() {
            let imports = self.import_names();
            if let Some((script, _)) = self.resolver.find_imported_global(&imports, function) {
                return self.build_call(
                    CallRecv::Static(script.clone()),
                    &script,
                    function,
                    args,
                    arg_names,
                    line,
                    None,
                );
            }
        }
        self.build_call(
            CallRecv::SelfObj,
            &owner,
            function,
            args,
            arg_names,
            line,
            None,
        )
    }

    fn import_names(&self) -> Vec<String> {
        self.ast
            .imports
            .iter()
            .map(|i| i.script_name.clone())
            .collect()
    }

    /// `obj.method(args)` / `Parent.method(args)` / `Script.method(args)`.
    fn lower_dot_call(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        line: u16,
        expected: Option<&PapyrusType>,
    ) -> PexValuePayload {
        // Array built-in methods compile to dedicated array opcodes, not CALLMETHOD.
        if matches!(self.type_of_expr(object), PapyrusType::Array(_)) {
            if let Some(v) = self.lower_array_method(object, method, args, line) {
                return v;
            }
        }
        match object {
            Expr::ParentExpr { .. } => {
                let parent = self.parent_name();
                self.build_call(CallRecv::Parent, &parent, method, args, &[], line, None)
            }
            // A bare name that is not a value but names a script → static call on
            // that script (e.g. `Utility.Wait(1.0)`, `Game.GetPlayer()`).
            Expr::NameExpr { name, .. } if self.is_static_call_name(name) => self.build_call(
                CallRecv::Static(name.clone()),
                name,
                method,
                args,
                &[],
                line,
                expected,
            ),
            _ => {
                let obj = self.lower_expr(object, line);
                let owner = type_name(&self.type_of_expr(object)).unwrap_or_default();
                self.build_call(CallRecv::Obj(obj), &owner, method, args, &[], line, None)
            }
        }
    }

    /// Array built-in methods (`Find`/`RFind`/`FindStruct`/`RFindStruct`/`Add`/
    /// `Insert`/`Remove`/`RemoveLast`/`Clear`). These lower to dedicated array
    /// opcodes whose first operand is the array and (for the value-returning
    /// search methods) whose second operand is a fresh `Int` result temp. The
    /// receiver and provided args are lowered post-order before the result temp
    /// is minted, matching stock temp numbering. Omitted optional args take their
    /// stock defaults (`Find`/`FindStruct` start at 0, the reverse variants at -1,
    /// `Add`/`Remove` count 1). Returns `None` for any non-array-builtin method so
    /// the caller falls through to a normal `CALLMETHOD`.
    fn lower_array_method(
        &mut self,
        object: &Expr,
        method: &str,
        args: &[Expr],
        line: u16,
    ) -> Option<PexValuePayload> {
        let m = method.to_ascii_lowercase();
        if !matches!(
            m.as_str(),
            "find"
                | "rfind"
                | "findstruct"
                | "rfindstruct"
                | "add"
                | "insert"
                | "remove"
                | "removelast"
                | "clear"
        ) {
            return None;
        }
        let arr = self.lower_expr(object, line);
        let lowered: Vec<PexValuePayload> = args.iter().map(|a| self.lower_expr(a, line)).collect();
        Some(match m.as_str() {
            "find" | "rfind" => {
                let elem = lowered.first().cloned()?;
                let start = lowered
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| int_value(if m == "find" { 0 } else { -1 }));
                let dest = self.alloc_temp(&PapyrusType::Int);
                let op = if m == "find" {
                    OP_ARRAYFINDELEMENT
                } else {
                    OP_ARRAYRFINDELEMENT
                };
                self.emit(op, vec![arr, ident(&dest), elem, start], line);
                ident(&dest)
            }
            "findstruct" | "rfindstruct" => {
                let member = lowered.first().cloned()?;
                let value = lowered.get(1).cloned()?;
                let start = lowered
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| int_value(if m == "findstruct" { 0 } else { -1 }));
                let dest = self.alloc_temp(&PapyrusType::Int);
                let op = if m == "findstruct" {
                    OP_ARRAYFINDSTRUCT
                } else {
                    OP_ARRAYRFINDSTRUCT
                };
                self.emit(op, vec![arr, ident(&dest), member, value, start], line);
                ident(&dest)
            }
            // The mutators are void: stock declares the shared `::nonevar` sentinel
            // for the discarded statement result even though the array opcode takes
            // no dest operand. Touch `nonevar()` so it's declared, then return it.
            "add" => {
                let elem = lowered.first().cloned()?;
                let count = lowered.get(1).cloned().unwrap_or_else(|| int_value(1));
                self.emit(OP_ARRAYADD, vec![arr, elem, count], line);
                self.nonevar()
            }
            "insert" => {
                let elem = lowered.first().cloned()?;
                let index = lowered.get(1).cloned()?;
                self.emit(OP_ARRAYINSERT, vec![arr, elem, index], line);
                self.nonevar()
            }
            "remove" => {
                let index = lowered.first().cloned()?;
                let count = lowered.get(1).cloned().unwrap_or_else(|| int_value(1));
                self.emit(OP_ARRAYREMOVE, vec![arr, index, count], line);
                self.nonevar()
            }
            "removelast" => {
                self.emit(OP_ARRAYREMOVELAST, vec![arr], line);
                self.nonevar()
            }
            "clear" => {
                self.emit(OP_ARRAYCLEAR, vec![arr], line);
                self.nonevar()
            }
            _ => unreachable!(),
        })
    }

    /// Lower a call: resolve the callee signature (return type + params with
    /// defaults) up the receiver's hierarchy, evaluate the provided args (cast
    /// to their param types), then fill omitted trailing params with their
    /// defaults (the stock compiler always materializes every parameter). The
    /// result lands in `::nonevar` for a void callee, else a fresh typed temp.
    fn build_call(
        &mut self,
        recv: CallRecv,
        owner_type: &str,
        method: &str,
        provided: &[Expr],
        arg_names: &[Option<String>],
        line: u16,
        ret_hint: Option<&PapyrusType>,
    ) -> PexValuePayload {
        let sig = self.resolve_callee(owner_type, method);
        let unresolved = sig.is_none();
        let (ret_decl, params, is_global) = match sig {
            Some(s) => (type_string_for_decl(&s.return_type), s.params, s.is_global),
            None => ("None".to_string(), Vec::new(), false),
        };

        // PCompiler's two-stage temp model: the TypeWalker numbers EVERY argument-
        // base temp before any parameter-cast temp, while PapyrusGen emits the
        // casts interleaved (base, cast, base, cast). So emit each needed cast in
        // place with a placeholder dest, then number the cast temps in argument
        // order once all bases are minted and back-patch (appends never shift
        // earlier indices). `Reg(GetPlayer(), GetActorRef(), n)` -> locals
        // [actor, actor, ScriptObject, ScriptObject], not the interleaved order.
        // Bind provided args to emitted-arg slots. Named args (`Name = expr`) bind
        // by parameter name — allowing out-of-order / gap-skipping — while unnamed
        // args fill the remaining slots positionally. Each slot's parameter index
        // equals its position. `None` = the parameter takes its default value.
        let use_names = !params.is_empty() && arg_names.iter().any(|n| n.is_some());
        let slot_src: Vec<Option<usize>> = if use_names {
            let mut slots: Vec<Option<usize>> = vec![None; params.len()];
            let mut next_pos = 0usize;
            for (i, nm) in arg_names.iter().enumerate() {
                match nm {
                    Some(name) => {
                        if let Some(j) = params
                            .iter()
                            .position(|p| p.name.eq_ignore_ascii_case(name))
                        {
                            slots[j] = Some(i);
                        }
                    }
                    None => {
                        while next_pos < slots.len() && slots[next_pos].is_some() {
                            next_pos += 1;
                        }
                        if next_pos < slots.len() {
                            slots[next_pos] = Some(i);
                            next_pos += 1;
                        }
                    }
                }
            }
            slots
        } else {
            // Positional: provided fill slots 0..n, then trailing params take their
            // default until one without a default (mirrors the original cutoff).
            let mut v: Vec<Option<usize>> = (0..provided.len()).map(Some).collect();
            for p in params.iter().skip(provided.len()) {
                if p.default.is_some() {
                    v.push(None);
                } else {
                    break;
                }
            }
            v
        };

        let none = PexValuePayload {
            value_type: VT_NONE,
            data: serde_json::Value::Null,
        };
        let mut args: Vec<PexValuePayload> = vec![none; slot_src.len()];
        // PCompiler's two-stage temp model: the TypeWalker numbers EVERY argument-
        // base temp before any parameter-cast temp, while PapyrusGen emits the
        // casts interleaved. Lower each provided base in slot order, emitting any
        // needed cast against a placeholder, then number the cast temps in slot
        // order once all bases are minted and back-patch.
        let mut deferred_casts: Vec<(usize, usize, PapyrusType)> = Vec::new();
        for (slot, src) in slot_src.iter().enumerate() {
            let Some(i) = src else { continue };
            let a = &provided[*i];
            let v = self.lower_expr(a, line);
            match params.get(slot) {
                Some(p) => {
                    let from = self.type_of_expr(a);
                    let to = parse_ty(&p.ty);
                    if from == to
                        || (type_eq_ci(&from, &to) && self.is_script_member_expr(a))
                        || to == PapyrusType::None
                        || (is_string_like(&from) && is_string_like(&to))
                    {
                        args[slot] = v;
                    } else {
                        let idx = self.instrs.len();
                        self.emit(OP_CAST, vec![ident(CAST_PLACEHOLDER), v], line);
                        deferred_casts.push((slot, idx, to));
                        args[slot] = ident(CAST_PLACEHOLDER);
                    }
                }
                None => args[slot] = v,
            }
        }
        for (slot, idx, to) in deferred_casts {
            let dest = self.alloc_temp(&to);
            self.instrs[idx].args[0] = ident(&dest);
            args[slot] = ident(&dest);
        }
        // Unprovided slots materialize their parameter's default VALUE: a constant
        // (incl. a negated numeric literal) folds to a literal operand rather than
        // emitting an INEG/FNEG temp.
        for (slot, src) in slot_src.iter().enumerate() {
            if src.is_none() {
                if let Some(d) = params.get(slot).and_then(|p| p.default.as_ref()) {
                    args[slot] = const_value(d).unwrap_or_else(|| self.lower_expr(d, line));
                }
            }
        }

        // A CustomEventName literal is emitted qualified as
        // `<declaringscript_lc>_<EventName>`. The declaring script is the akSender
        // (1st arg) type for (Un)RegisterForCustomEvent and the receiver type for
        // SendCustomEvent.
        let custom_event = match method.to_ascii_lowercase().as_str() {
            "registerforcustomevent" | "unregisterforcustomevent" => provided
                .first()
                .map(|e| (1usize, type_name(&self.type_of_expr(e)).unwrap_or_default())),
            "sendcustomevent" => Some((0usize, owner_type.to_string())),
            _ => None,
        };
        if let Some((idx, owner)) = custom_event {
            if !owner.is_empty() {
                if let Some(arg) = args.get_mut(idx) {
                    if arg.value_type == VT_STRING {
                        if let serde_json::Value::String(s) = &arg.data {
                            arg.data = serde_json::Value::String(format!(
                                "{}_{}",
                                owner.to_ascii_lowercase(),
                                s
                            ));
                        }
                    }
                }
            }
        }

        let dest = if ret_decl == "None" {
            match ret_hint.filter(|_| unresolved) {
                Some(ty) if *ty != PapyrusType::None => ident(&self.alloc_temp(ty)),
                _ => self.nonevar(),
            }
        } else {
            ident(&self.alloc_temp(&parse_ty(&ret_decl)))
        };
        let argc = int_value(args.len() as i64);

        let m = self.intern_ident(method);
        let mut head = match &recv {
            CallRecv::Parent => vec![ident(&m), dest.clone(), argc],
            CallRecv::Obj(obj) => vec![ident(&m), obj.clone(), dest.clone(), argc],
            CallRecv::SelfObj if is_global => {
                vec![
                    ident(&self.emitted_script_name()),
                    ident(&m),
                    dest.clone(),
                    argc,
                ]
            }
            CallRecv::SelfObj => vec![ident(&m), ident("self"), dest.clone(), argc],
            CallRecv::Static(script) => {
                let recv_ty = cased_type(script, &self.parent_name());
                vec![ident(&recv_ty), ident(&m), dest.clone(), argc]
            }
        };
        head.extend(args);
        let opcode = match recv {
            CallRecv::Parent => OP_CALLPARENT,
            CallRecv::Obj(_) => OP_CALLMETHOD,
            CallRecv::SelfObj if is_global => OP_CALLSTATIC,
            CallRecv::SelfObj => OP_CALLMETHOD,
            CallRecv::Static(_) => OP_CALLSTATIC,
        };
        self.emit(opcode, head, line);
        dest
    }

    /// Resolve a method/function up the owner's hierarchy, returning its return
    /// type, params (with defaults) and global-ness. Same-script callees come
    /// straight from the AST; others are resolved by parsing their source.
    fn resolve_callee(&self, owner_type: &str, method: &str) -> Option<CalleeSig> {
        // The script being compiled isn't in the import path, so resolve its own
        // methods from the AST and walk its ancestors starting from its parent.
        let chain: Vec<String> = if owner_type.eq_ignore_ascii_case(&self.ast.name) {
            if let Some(f) = self.lookup_function(method) {
                return Some(CalleeSig::from_def(f));
            }
            match self.ast.parent.as_deref() {
                Some(p) if !p.is_empty() => {
                    let mut c = vec![p.to_string()];
                    c.extend(self.resolver.get_hierarchy(p));
                    c
                }
                _ => Vec::new(),
            }
        } else {
            let mut c = vec![owner_type.to_string()];
            c.extend(self.resolver.get_hierarchy(owner_type));
            c
        };
        for script in chain {
            if let Some(ast) = self.resolver.parsed(&script) {
                if let Some(f) = ast
                    .functions
                    .iter()
                    .find(|f| f.name.eq_ignore_ascii_case(method))
                {
                    return Some(CalleeSig::from_def(f));
                }
            }
        }
        None
    }

    fn is_value_name(&self, name: &str) -> bool {
        let k = name.to_lowercase();
        self.scope.contains_key(&k) || self.members.contains_key(&k)
    }

    fn is_static_call_name(&self, name: &str) -> bool {
        !self.is_value_name(name) && (name.contains(':') || self.resolver.script_exists(name))
    }

    fn is_unresolved_static_dot_call(&self, expr: &Expr) -> bool {
        match expr {
            Expr::DotCallExpr { object, method, .. } => match object.as_ref() {
                Expr::NameExpr { name, .. } if self.is_static_call_name(name) => {
                    self.resolve_callee(name, method).is_none()
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn is_script_member_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::NameExpr { name, .. } => {
                let key = name.to_lowercase();
                self.members.contains_key(&key) && !self.scope.contains_key(&key)
            }
            _ => false,
        }
    }

    fn lookup_function(&self, name: &str) -> Option<&'a FunctionDef> {
        self.ast
            .functions
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// Map a bare name to its emitted identifier: a shadowed local resolves to
    /// its mangled name, an auto-property to its backing variable, else itself.
    fn resolve_name(&self, name: &str) -> String {
        // The `self` keyword is emitted lowercased (stock compiler).
        if name.eq_ignore_ascii_case("self") {
            return "self".to_string();
        }
        let lc = name.to_lowercase();
        if let Some(alias) = self.local_aliases.get(&lc) {
            return alias.clone();
        }
        if let Some(decl) = self.local_names.get(&lc) {
            return decl.clone();
        }
        self.prop_vars
            .get(&lc)
            .cloned()
            .or_else(|| self.ident_casing.get(&lc).cloned())
            .unwrap_or_else(|| name.to_string())
    }

    /// Register a local declaration, returning the name to emit. The first
    /// occurrence in a function keeps its source name; a later re-declaration of
    /// the same (lower-cased) name is shadow-mangled to `::mangled_<name>_<N>`
    /// (PapyrusGen.MangleFunctionVariables), with the alias scoped to the current
    /// block so references resolve to the mangled name until the block exits.
    fn declare_local(&mut self, name: &str) -> String {
        let lc = name.to_lowercase();
        if self.declared_locals.contains(&lc) {
            let mangled = format!("::mangled_{}_{}", lc, self.mangle_counter);
            self.mangle_counter += 1;
            let old = self.local_aliases.insert(lc.clone(), mangled.clone());
            if let Some(frame) = self.block_stack.last_mut() {
                frame.mangle_restores.push((lc, old));
            }
            mangled
        } else {
            self.declared_locals.insert(lc.clone());
            self.local_names.insert(lc, name.to_string());
            name.to_string()
        }
    }

    /// The shared `::nonevar` (None) sentinel, allocated lazily at function root.
    fn nonevar(&mut self) -> PexValuePayload {
        if !self.nonevar_used {
            self.nonevar_used = true;
            // The none sentinel is minted in the current code block in encounter
            // order, like any temp (PCompiler GenerateTempVariable) — NOT hoisted
            // to the function root.
            self.block_stack
                .last_mut()
                .unwrap()
                .temps
                .push(PexLocalPayload {
                    name: "::nonevar".to_string(),
                    ty: "None".to_string(),
                });
        }
        ident("::nonevar")
    }

    fn lower_binary(
        &mut self,
        node: &Expr,
        left: &Expr,
        op: &str,
        right: &Expr,
        line: u16,
    ) -> PexValuePayload {
        let result_ty = self.type_of_node(node);

        let lt = self.type_of_expr(left);
        let rt = self.type_of_expr(right);
        let (left_v, right_v) = self.lower_binop_operands(left, lt, right, rt, &result_ty, line);

        let opcode = arith_opcode(op, &result_ty);
        let dest = self.alloc_temp(&result_ty);
        self.emit(opcode, vec![ident(&dest), left_v, right_v], line);
        ident(&dest)
    }

    /// Comparison → a `Bool` temp. `!=` is `CMP_EQ` followed by `NOT`. Operands
    /// are promoted to their common numeric type before comparing.
    fn lower_comparison(
        &mut self,
        op: &str,
        left: &Expr,
        right: &Expr,
        line: u16,
    ) -> PexValuePayload {
        let lt = self.type_of_expr(left);
        let rt = self.type_of_expr(right);
        let common = common_compare_type(&lt, &rt);
        let (lv, rv) = self.lower_binop_operands(left, lt, right, rt, &common, line);
        let dest = self.alloc_temp(&PapyrusType::Bool);
        let opcode = match op {
            "==" | "!=" => OP_CMP_EQ,
            "<" => OP_CMP_LT,
            "<=" => OP_CMP_LE,
            ">" => OP_CMP_GT,
            ">=" => OP_CMP_GE,
            _ => OP_CMP_EQ,
        };
        self.emit(opcode, vec![ident(&dest), lv, rv], line);
        if op == "!=" {
            self.emit(OP_NOT, vec![ident(&dest), ident(&dest)], line);
        }
        ident(&dest)
    }

    /// Short-circuit `&&` / `||`: a single `Bool` target temp; the left side is
    /// moved in (always via `CAST`), a conditional jump skips the right side,
    /// and the right side is moved into the same temp.
    fn lower_logical(&mut self, op: &str, left: &Expr, right: &Expr, line: u16) -> PexValuePayload {
        // The type-walker mints the result temp in POST-ORDER (after both
        // operands' temps), but the short-circuit CAST/jump need it before the
        // right operand. Emit those against a placeholder, lower both operands
        // first (so their temps are numbered ahead of the result), then mint the
        // result temp and back-patch the placeholder by instruction index —
        // appends never shift the earlier indices.
        let placeholder = ident("::__logical_dest");
        let lv = self.lower_expr(left, line);
        let cast_l = self.instrs.len();
        self.emit(OP_CAST, vec![placeholder.clone(), lv], line);
        let end = self.new_label();
        let jop = if op == "&&" { OP_JMPF } else { OP_JMPT };
        let jump = self.instrs.len();
        self.emit_jump(jop, Some(placeholder), end, line);
        let rv = self.lower_expr(right, line);
        let dest = self.alloc_temp(&PapyrusType::Bool);
        self.emit(OP_CAST, vec![ident(&dest), rv], line);
        self.place_label(end);
        self.instrs[cast_l].args[0] = ident(&dest);
        self.instrs[jump].args[0] = ident(&dest);
        ident(&dest)
    }

    /// `!a` → `NOT` (Bool); `-a` → `INEG`/`FNEG` by operand type.
    fn lower_unary(&mut self, node: &Expr, op: &str, operand: &Expr, line: u16) -> PexValuePayload {
        let ov = self.lower_expr(operand, line);
        match op {
            "!" => {
                let dest = self.alloc_temp(&PapyrusType::Bool);
                self.emit(OP_NOT, vec![ident(&dest), ov], line);
                ident(&dest)
            }
            "-" => {
                // A negated numeric literal in an EXPRESSION is NOT constant-folded
                // by stock (no -optimize): it emits INEG/FNEG into a temp (`-1` ->
                // INEG temp,1). Folding only happens for materialized default values
                // (see `const_value`), not here.
                let ty = self.type_of_node(node);
                let opcode = if ty == PapyrusType::Float {
                    OP_FNEG
                } else {
                    OP_INEG
                };
                let dest = self.alloc_temp(&ty);
                self.emit(opcode, vec![ident(&dest), ov], line);
                ident(&dest)
            }
            _ => ov,
        }
    }

    // --- jump / label machinery ----------------------------------------------

    fn new_label(&mut self) -> u32 {
        let l = self.label_counter;
        self.label_counter += 1;
        l
    }

    fn place_label(&mut self, id: u32) {
        self.labels.insert(id, self.instrs.len());
    }

    /// Emit a jump (`JMP`/`JMPT`/`JMPF`) with a placeholder offset to back-patch.
    fn emit_jump(&mut self, opcode: u8, cond: Option<PexValuePayload>, target: u32, line: u16) {
        let mut args = Vec::new();
        if let Some(c) = cond {
            args.push(c);
        }
        args.push(int_value(0)); // placeholder offset, resolved in `resolve_jumps`
        let idx = self.instrs.len();
        self.instrs.push(PexInstructionPayload { opcode, args });
        self.debug_lines.push(line);
        self.pending_jumps.push((idx, target));
    }

    /// Back-patch every pending jump to a relative offset (`target − self`).
    fn resolve_jumps(&mut self) {
        for (idx, target) in std::mem::take(&mut self.pending_jumps) {
            let dest = self.labels[&target] as i64;
            let offset = dest - idx as i64;
            let args = &mut self.instrs[idx].args;
            let last = args.len() - 1;
            args[last] = int_value(offset);
        }
    }

    /// Source line of the most recently emitted instruction (used for the
    /// synthesized trailing/back jumps, which the exe tags with the body's last
    /// line).
    fn last_line(&self) -> u16 {
        self.debug_lines.last().copied().unwrap_or(0)
    }

    /// Insert a `CAST` into a fresh temp when `from` ≠ `to`. Identity casts are
    /// elided (no instruction).
    fn cast_to(
        &mut self,
        value: PexValuePayload,
        from: PapyrusType,
        to: &PapyrusType,
        line: u16,
    ) -> PexValuePayload {
        if &from == to || *to == PapyrusType::None {
            return value;
        }
        // CustomEventName is `string` at the bytecode level, so coercing between
        // the two is identity — stock passes the event-name literal directly to
        // RegisterForCustomEvent / SendCustomEvent with no CAST (and no temp).
        if is_string_like(&from) && is_string_like(to) {
            return value;
        }
        let dest = self.alloc_temp(to);
        self.emit(OP_CAST, vec![ident(&dest), value], line);
        ident(&dest)
    }

    /// Lower two binary operands to `common`. The type-walker numbers each
    /// operand's cast temp AFTER both operand bases, while PapyrusGen emits the
    /// CAST in operand position. Emit each cast against a placeholder dest, then
    /// number the cast temps once both bases exist, back-patching the dest.
    fn lower_binop_operands(
        &mut self,
        left: &Expr,
        lt: PapyrusType,
        right: &Expr,
        rt: PapyrusType,
        common: &PapyrusType,
        line: u16,
    ) -> (PexValuePayload, PexValuePayload) {
        let lv_base = self.lower_expr(left, line);
        let l_idx = if would_cast(&lt, common) {
            let i = self.instrs.len();
            self.emit(
                OP_CAST,
                vec![ident(CAST_PLACEHOLDER), lv_base.clone()],
                line,
            );
            Some(i)
        } else {
            None
        };
        let rv_base = self.lower_expr(right, line);
        let r_idx = if would_cast(&rt, common) {
            let i = self.instrs.len();
            self.emit(
                OP_CAST,
                vec![ident(CAST_PLACEHOLDER), rv_base.clone()],
                line,
            );
            Some(i)
        } else {
            None
        };
        let lv = match l_idx {
            Some(i) => {
                let t = self.alloc_temp(common);
                self.instrs[i].args[0] = ident(&t);
                ident(&t)
            }
            None => lv_base,
        };
        let rv = match r_idx {
            Some(i) => {
                let t = self.alloc_temp(common);
                self.instrs[i].args[0] = ident(&t);
                ident(&t)
            }
            None => rv_base,
        };
        (lv, rv)
    }

    fn alloc_temp(&mut self, ty: &PapyrusType) -> String {
        let key = ty.to_string().to_lowercase();
        // Reuse a freed temp of the exact same type (FIFO), else mint a new one.
        if let Some(list) = self.unused_temps.get_mut(&key) {
            if !list.is_empty() {
                return list.remove(0);
            }
        }
        let name = format!("::temp{}", self.temp_counter);
        self.temp_counter += 1;
        let frame = self.block_stack.last_mut().unwrap();
        frame.temps.push(PexLocalPayload {
            name: name.clone(),
            ty: ty.to_string(),
        });
        frame.ptemp.push((name.clone(), key));
        name
    }

    /// Return the current (innermost) block's temps to the function-global
    /// free-list (PCompiler `MarkAllTempVarsAsUnused(codeBlock.Peek().ptempVars)`,
    /// run after each statement). A temp already queued is not duplicated.
    fn mark_all_temps_unused(&mut self) {
        let ptemp = self.block_stack.last().unwrap().ptemp.clone();
        for (name, key) in ptemp {
            let list = self.unused_temps.entry(key).or_default();
            if !list.contains(&name) {
                list.push(name);
            }
        }
    }

    fn enter_block(&mut self) {
        self.block_stack.push(BlockFrame::default());
    }

    /// Pop the current block and attach its flattened locals to the parent, at
    /// the parent's current source position.
    fn exit_block(&mut self) {
        let mut frame = self.block_stack.pop().unwrap();
        for (lc, old) in std::mem::take(&mut frame.mangle_restores).into_iter().rev() {
            match old {
                Some(v) => self.local_aliases.insert(lc, v),
                None => self.local_aliases.remove(&lc),
            };
        }
        let flat = frame.flatten();
        self.block_stack
            .last_mut()
            .unwrap()
            .entries
            .push(LocalEntry::Block(flat));
    }

    fn emit(&mut self, opcode: u8, args: Vec<PexValuePayload>, line: u16) {
        self.instrs.push(PexInstructionPayload { opcode, args });
        self.debug_lines.push(line);
    }

    // --- type lookups ---------------------------------------------------------

    fn type_of_expr(&self, expr: &Expr) -> PapyrusType {
        let tc_ty = self
            .expr_ids
            .get(&(expr as *const Expr as usize))
            .and_then(|id| self.tc.types.get(*id))
            .cloned();
        match tc_ty {
            Some(t) if t != PapyrusType::None => t,
            // The type checker runs with an empty script DB during the golden
            // loop, so same-script calls / `.Length` come back unresolved; fall
            // back to a codegen-local inference for the kinds we lower.
            _ => self.infer_type(expr),
        }
    }

    fn infer_type(&self, expr: &Expr) -> PapyrusType {
        match expr {
            Expr::LiteralExpr { ty, .. } => parse_ty(ty),
            Expr::NameExpr { name, .. } => {
                let key = name.to_lowercase();
                self.scope
                    .get(&key)
                    .or_else(|| self.members.get(&key))
                    .cloned()
                    .unwrap_or(PapyrusType::None)
            }
            Expr::CallExpr { function, .. } if function.starts_with("new ") => {
                parse_ty(&function["new ".len()..])
            }
            Expr::CallExpr { function, .. } => match self.lookup_function(function) {
                Some(f) => {
                    let s = type_string_for_decl(&f.return_type);
                    if s == "None" {
                        PapyrusType::None
                    } else {
                        parse_ty(&s)
                    }
                }
                None => PapyrusType::None,
            },
            Expr::DotExpr { object, member, .. }
                if member.eq_ignore_ascii_case("Length")
                    && matches!(self.type_of_expr(object), PapyrusType::Array(_)) =>
            {
                PapyrusType::Int
            }
            // `structVal.member` — a same-script struct member access.
            Expr::DotExpr { object, member, .. }
                if self
                    .struct_member_ty(&self.type_of_expr(object).to_string(), member)
                    .is_some() =>
            {
                self.struct_member_ty(&self.type_of_expr(object).to_string(), member)
                    .unwrap()
            }
            // `obj.Property` — resolve the property's type up the receiver script's
            // hierarchy (needed so PROPGET/PROPSET temps get the right type).
            Expr::DotExpr { object, member, .. } => {
                let obj_ty = self.type_of_expr(object);
                type_name(&obj_ty)
                    .and_then(|script| {
                        self.resolver
                            .get_hierarchy(&script)
                            .into_iter()
                            .find_map(|anc| {
                                self.resolver
                                    .get_properties(&anc)
                                    .into_iter()
                                    .find(|p| p.name.eq_ignore_ascii_case(member.as_str()))
                                    .map(|p| parse_ty(&p.ty))
                            })
                    })
                    .unwrap_or(PapyrusType::None)
            }
            _ => PapyrusType::None,
        }
    }

    /// Members of a same-script struct named by `type_name` (a bare `Struct`
    /// name or the qualified `thisscript#struct` spelling). Cross-script structs
    /// (`otherscript#struct`) are not yet resolved.
    fn struct_members(&self, type_name: &str) -> Option<&[StructMemberDef]> {
        let base = type_name.trim_end_matches("[]");
        let (script, sname) = match base.split_once('#') {
            Some((sc, sn)) => (Some(sc), sn),
            None => (None, base),
        };
        if let Some(sc) = script {
            if !sc.eq_ignore_ascii_case(&self.ast.name) {
                return None;
            }
        }
        self.ast
            .structs
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(sname))
            .map(|s| s.members.as_slice())
    }

    fn struct_member_ty(&self, type_name: &str, member: &str) -> Option<PapyrusType> {
        self.struct_members(type_name)?
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(member))
            .map(|m| parse_ty(&m.ty))
    }

    fn type_of_node(&self, node: &Expr) -> PapyrusType {
        self.type_of_expr(node)
    }

    // --- string table ---------------------------------------------------------

    /// Build a deterministic string table: the user-flag names first (keeping
    /// the slice byte-identical), then every content string in first-use order.
    /// Parity does not depend on this order — the golden comparison sorts it.
    fn build_string_table(
        &self,
        user_flags: &[PexUserFlagPayload],
        object: &PexObjectPayload,
        debug: Option<&PexDebugInfoPayload>,
    ) -> Vec<String> {
        let mut it = Interner::default();
        for uf in user_flags {
            it.add(&uf.name);
        }
        it.add(&object.name);
        it.add(&object.parent);
        it.add(&object.docstring);
        it.add(&object.auto_state);
        for v in &object.variables {
            it.add(&v.name);
            it.add(&v.ty);
            // A String-typed initial value references a string-table entry.
            if v.data.value_type == VT_STRING {
                if let Some(s) = v.data.data.as_str() {
                    it.add(s);
                }
            }
        }
        for p in &object.properties {
            it.add(&p.name);
            it.add(&p.ty);
            it.add(&p.docstring);
            it.add(&p.auto_var);
            // Handler getter/setter functions carry no stored name (the reader
            // fabricates `get_`/`set_`), so intern only their body strings.
            if let Some(g) = &p.getter {
                intern_function_strings(&mut it, g);
            }
            if let Some(s) = &p.setter {
                intern_function_strings(&mut it, s);
            }
        }
        for s in &object.structs {
            it.add(&s.name);
            for m in &s.members {
                it.add(&m.ty);
                it.add(&m.name);
                it.add(&m.docstring);
                // A string-typed member default lives in the string table too.
                if m.data.value_type == VT_STRING {
                    if let Some(s) = m.data.data.as_str() {
                        it.add(s);
                    }
                }
            }
        }
        for st in &object.states {
            it.add(&st.name);
            for f in &st.functions {
                it.add(&f.name);
                intern_function_strings(&mut it, f);
            }
        }
        if let Some(d) = debug {
            for f in &d.functions {
                it.add(&f.object_name);
                it.add(&f.state_name);
                it.add(&f.function_name);
            }
            for g in &d.property_groups {
                it.add(&g.object_name);
                it.add(&g.group_name);
                it.add(&g.docstring);
                for n in &g.property_names {
                    it.add(n);
                }
            }
        }
        it.order
    }
}

/// Receiver kind for a lowered call, selecting the call opcode + fixed operands.
enum CallRecv {
    /// `CALLMETHOD … self …` (or `CALLSTATIC <thisscript> …` if the callee is
    /// a same-script global).
    SelfObj,
    /// `CALLSTATIC <script> …`.
    Static(String),
    /// `CALLMETHOD … <obj> …`.
    Obj(PexValuePayload),
    /// `CALLPARENT …`.
    Parent,
}

/// Resolved callee signature for return-type + default-argument handling.
struct CalleeSig {
    return_type: String,
    params: Vec<Parameter>,
    is_global: bool,
}

impl CalleeSig {
    fn from_def(f: &FunctionDef) -> Self {
        CalleeSig {
            return_type: f.return_type.clone(),
            params: f.params.clone(),
            is_global: f.is_global,
        }
    }
}

fn type_name(ty: &PapyrusType) -> Option<String> {
    match ty {
        PapyrusType::Object(n) | PapyrusType::Struct(n) => Some(n.clone()),
        _ => None,
    }
}

/// Canonicalize a payload for order-insensitive semantic comparison: zero the
/// §5 identity fields and sort every region the stock compiler emits in per-
/// process `Hashtable` order (string table, user flags, object property and
/// variable lists, per-state function lists, debug function list). Everything
/// else (instructions, operands, temps, locals order, debug line numbers,
/// string-table *indices*, property_names) is deterministic and left intact.
pub fn canonicalize(p: &mut PexFilePayload) {
    p.compilation_time = 0;
    p.source_filename.clear();
    p.username.clear();
    p.machine_name.clear();
    p.string_table.sort();
    p.user_flags.sort_by(|a, b| a.name.cmp(&b.name));
    if let Some(d) = p.debug_info.as_mut() {
        d.modification_time = 0;
        d.functions.sort_by(|a, b| {
            (&a.object_name, &a.state_name, &a.function_name).cmp(&(
                &b.object_name,
                &b.state_name,
                &b.function_name,
            ))
        });
    }
    for o in &mut p.objects {
        o.variables.sort_by(|a, b| a.name.cmp(&b.name));
        o.properties.sort_by(|a, b| a.name.cmp(&b.name));
        // The struct member LIST permutes via the same randomized Hashtable (the
        // debug `struct_orders` keeps source order); sort by name to compare
        // semantically.
        o.structs.sort_by(|a, b| a.name.cmp(&b.name));
        for s in &mut o.structs {
            s.members.sort_by(|a, b| a.name.cmp(&b.name));
        }
        // Stock orders the per-object state list via a randomized .NET Hashtable
        // (same as the per-state function list), so sort by name to compare
        // semantically. The empty-named default state sorts to index 0.
        o.states.sort_by(|a, b| a.name.cmp(&b.name));
        for s in &mut o.states {
            s.functions.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }
}

/// A declared local or a nested block's already-flattened locals, kept in
/// source order within a block.
enum LocalEntry {
    Decl(PexLocalPayload),
    Block(Vec<PexLocalPayload>),
}

#[derive(Default)]
struct BlockFrame {
    temps: Vec<PexLocalPayload>,
    entries: Vec<LocalEntry>,
    /// Temps minted while THIS block is innermost, as (name, lowercased-type),
    /// in allocation order (PCompiler per-codeBlock `ptempVars`). Returned to
    /// the function-global free-list when a statement in this block completes.
    ptemp: Vec<(String, String)>,
    /// Shadow-alias overrides this block installed, as (lowercased-name,
    /// previous alias) — replayed in reverse on block exit to restore scope.
    mangle_restores: Vec<(String, Option<String>)>,
}

impl BlockFrame {
    /// This block's temps first, then its statements (declared locals and
    /// nested blocks) in source order.
    fn flatten(self) -> Vec<PexLocalPayload> {
        let mut out = self.temps;
        for e in self.entries {
            match e {
                LocalEntry::Decl(l) => out.push(l),
                LocalEntry::Block(v) => out.extend(v),
            }
        }
        out
    }
}

#[derive(Default)]
struct Interner {
    order: Vec<String>,
    seen: HashSet<String>,
}

impl Interner {
    fn add(&mut self, s: &str) {
        if self.seen.insert(s.to_string()) {
            self.order.push(s.to_string());
        }
    }
}

/// Intern a function's content strings (return type, docstring, params, locals,
/// instruction operands) — but NOT its name, which the caller handles (state
/// functions store a name; property handlers do not).
fn intern_function_strings(it: &mut Interner, f: &PexFunctionPayload) {
    it.add(&f.return_type);
    it.add(&f.docstring);
    for p in &f.params {
        it.add(&p.name);
        it.add(&p.ty);
    }
    for l in &f.locals {
        it.add(&l.name);
        it.add(&l.ty);
    }
    for ins in &f.instructions {
        for a in &ins.args {
            if (a.value_type == VT_IDENT || a.value_type == VT_STRING) && a.data.is_string() {
                it.add(a.data.as_str().unwrap());
            }
        }
    }
}

// --- helpers ------------------------------------------------------------------

fn ident(name: &str) -> PexValuePayload {
    PexValuePayload {
        value_type: VT_IDENT,
        data: serde_json::Value::String(name.to_string()),
    }
}

fn int_value(n: i64) -> PexValuePayload {
    PexValuePayload {
        value_type: VT_INT,
        data: serde_json::json!(n),
    }
}

fn default_local_value(ty: &PapyrusType) -> Option<PexValuePayload> {
    match ty {
        PapyrusType::Int => Some(int_value(0)),
        PapyrusType::Float => Some(PexValuePayload {
            value_type: VT_FLOAT,
            data: serde_json::json!(0.0),
        }),
        PapyrusType::Bool => Some(PexValuePayload {
            value_type: VT_BOOL,
            data: serde_json::json!(false),
        }),
        PapyrusType::String => Some(PexValuePayload {
            value_type: VT_STRING,
            data: serde_json::json!(""),
        }),
        _ => None,
    }
}

/// Evaluate a constant initializer (a literal or a negated numeric literal) to
/// its `.pex` value. `-1` is a negated literal, not an INEG operation, so a
/// `= -1` default materializes the folded constant. Non-constant exprs yield None.
fn const_value(expr: &Expr) -> Option<PexValuePayload> {
    match expr {
        Expr::LiteralExpr { value, ty, .. } => Some(literal_value(value, ty)),
        Expr::UnaryExpr { op, operand, .. } if op == "-" => match operand.as_ref() {
            Expr::LiteralExpr {
                value: LiteralValue::Int(n),
                ..
            } => Some(int_value(-n)),
            Expr::LiteralExpr {
                value: LiteralValue::Float(f),
                ..
            } => Some(PexValuePayload {
                value_type: VT_FLOAT,
                data: serde_json::json!(-f),
            }),
            _ => None,
        },
        _ => None,
    }
}

fn is_comparison(op: &str) -> bool {
    matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=")
}

/// Common type the two operands of a comparison are promoted to before the
/// compare opcode runs (Int widens to Float; otherwise the shared type).
fn common_compare_type(lt: &PapyrusType, rt: &PapyrusType) -> PapyrusType {
    use PapyrusType::*;
    if lt == rt {
        return lt.clone();
    }
    match (lt, rt) {
        // Int/Float numeric promotion.
        (Float, _) | (_, Float) => Float,
        // Otherwise the right operand is cast to the left operand's type
        // (e.g. `Bool == Int` compares as Bool, not Int).
        _ => lt.clone(),
    }
}

fn literal_value(value: &LiteralValue, ty: &str) -> PexValuePayload {
    match (value, ty) {
        (LiteralValue::Int(n), _) => PexValuePayload {
            value_type: VT_INT,
            data: serde_json::json!(n),
        },
        (LiteralValue::Float(f), _) => PexValuePayload {
            value_type: VT_FLOAT,
            data: serde_json::json!(f),
        },
        (LiteralValue::Str(s), _) => PexValuePayload {
            value_type: VT_STRING,
            data: serde_json::Value::String(s.clone()),
        },
        (LiteralValue::Bool(b), _) => PexValuePayload {
            value_type: VT_BOOL,
            data: serde_json::json!(b),
        },
        (LiteralValue::Null, _) => PexValuePayload {
            value_type: VT_NONE,
            data: serde_json::Value::Null,
        },
    }
}

/// `.pex` type string for a source-level type, mapping void/empty to `None`.
fn has_flag(flags: &[String], name: &str) -> bool {
    flags.iter().any(|f| f.eq_ignore_ascii_case(name))
}

enum FnOrEvent<'a> {
    Fn(&'a FunctionDef),
    Event(&'a EventDef),
}

impl FnOrEvent<'_> {
    fn line(&self) -> u32 {
        match self {
            FnOrEvent::Fn(f) => f.pos.line,
            FnOrEvent::Event(e) => e.pos.line,
        }
    }
}

/// Merge a state's functions and events into source order (by declaration line)
/// so the script-global temp counter numbers them as the stock compiler does.
fn source_ordered<'a>(funcs: &'a [FunctionDef], events: &'a [EventDef]) -> Vec<FnOrEvent<'a>> {
    let mut items: Vec<FnOrEvent<'a>> = funcs
        .iter()
        .map(FnOrEvent::Fn)
        .chain(events.iter().map(FnOrEvent::Event))
        .collect();
    items.sort_by_key(|it| match it {
        FnOrEvent::Fn(f) => f.pos.line,
        FnOrEvent::Event(e) => e.pos.line,
    });
    items
}

/// FO4 mangles a remote/custom-event handler `Event Source.EventName(...)` into
/// a function named `::remote_Source_EventName`. Plain events keep their name.
fn remote_event_name(name: &str) -> String {
    if name.contains('.') {
        format!("::remote_{}", name.replace('.', "_"))
    } else {
        name.to_string()
    }
}

fn type_string_for_decl(ty: &str) -> String {
    let t = ty.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("none") {
        "None".to_string()
    } else {
        parse_ty(t).to_string()
    }
}

/// Rewrite a type string to the stock compiler's casing: object type names are
/// lowercased, EXCEPT the script's parent type, which keeps its declared
/// (canonical) case and is reused for in-script references to that same type.
/// Primitives (Int/Float/Bool/String/Var/None) always keep canonical case.
fn cased_type(ty: &str, parent: &str) -> String {
    if let Some(base) = ty.strip_suffix("[]") {
        return format!("{}[]", cased_type(base, parent));
    }
    match ty.to_ascii_lowercase().as_str() {
        "int" => "Int".to_string(),
        "float" => "Float".to_string(),
        "bool" => "Bool".to_string(),
        "string" => "String".to_string(),
        "var" => "Var".to_string(),
        "none" => "None".to_string(),
        // ScriptObject is the implicit root of every script's parent chain, so
        // stock keeps it canonical-cased like a declared parent (not lowercased).
        "scriptobject" => "ScriptObject".to_string(),
        _ if !parent.is_empty() && ty.eq_ignore_ascii_case(parent) => parent.to_string(),
        _ => ty.to_ascii_lowercase(),
    }
}

/// Rewrite a same-script struct type reference to the stock spelling:
/// `script#Struct`. Namespaced object types also contain `:`, so they must stay
/// colon-separated unless we have proved they are structs.
fn qualify_struct(
    ty: &str,
    script: &str,
    structs: &HashSet<String>,
    resolver: &SourceResolver,
) -> String {
    if let Some(base) = ty.strip_suffix("[]") {
        return format!("{}[]", qualify_struct(base, script, structs, resolver));
    }
    if let Some((owner, member)) = ty.rsplit_once(':') {
        if resolver.has_struct(owner, member) {
            return format!("{owner}#{member}");
        }
    }
    if structs.contains(ty) {
        return format!("{}#{}", script.to_ascii_lowercase(), ty);
    }
    ty.to_string()
}

fn rw_ty(
    ty: &str,
    parent: &str,
    script: &str,
    structs: &HashSet<String>,
    resolver: &SourceResolver,
) -> String {
    qualify_struct(&cased_type(ty, parent), script, structs, resolver)
}

fn case_fn_types(
    f: &mut PexFunctionPayload,
    parent: &str,
    script: &str,
    structs: &HashSet<String>,
    resolver: &SourceResolver,
) {
    f.return_type = rw_ty(&f.return_type, parent, script, structs, resolver);
    for p in &mut f.params {
        p.ty = rw_ty(&p.ty, parent, script, structs, resolver);
    }
    for l in &mut f.locals {
        l.ty = rw_ty(&l.ty, parent, script, structs, resolver);
    }
}

/// Apply the stock compiler's object-type-name lowercasing + struct-type
/// qualification across every type string in the object (properties, variables,
/// params, locals, return types). `structs` is the set of lowercased same-script
/// struct names.
fn apply_type_name_case(
    obj: &mut PexObjectPayload,
    structs: &HashSet<String>,
    resolver: &SourceResolver,
) {
    let parent = obj.parent.clone();
    let script = obj.name.clone();
    for v in &mut obj.variables {
        v.ty = rw_ty(&v.ty, &parent, &script, structs, resolver);
    }
    for p in &mut obj.properties {
        p.ty = rw_ty(&p.ty, &parent, &script, structs, resolver);
        if let Some(g) = p.getter.as_mut() {
            case_fn_types(g, &parent, &script, structs, resolver);
        }
        if let Some(s) = p.setter.as_mut() {
            case_fn_types(s, &parent, &script, structs, resolver);
        }
    }
    for st in &mut obj.states {
        for f in &mut st.functions {
            case_fn_types(f, &parent, &script, structs, resolver);
        }
    }
}

/// Parse a source type string into the canonical `PapyrusType` (so casing /
/// array suffix match the type checker's spelling).
/// `String` and the compile-time-only `CustomEventName` are the same runtime
/// type, so coercion between them emits no instruction.
fn is_string_like(ty: &PapyrusType) -> bool {
    matches!(ty, PapyrusType::String)
        || matches!(ty, PapyrusType::Object(n)
            if n.eq_ignore_ascii_case("customeventname") || n.eq_ignore_ascii_case("scripteventname"))
}

fn type_eq_ci(a: &PapyrusType, b: &PapyrusType) -> bool {
    match (a, b) {
        (PapyrusType::Object(x), PapyrusType::Object(y))
        | (PapyrusType::Struct(x), PapyrusType::Struct(y)) => x.eq_ignore_ascii_case(y),
        (PapyrusType::Array(x), PapyrusType::Array(y)) => type_eq_ci(x, y),
        _ => a == b,
    }
}

/// Whether `cast_to` would emit a CAST instruction (mirrors its elision rules).
fn would_cast(from: &PapyrusType, to: &PapyrusType) -> bool {
    from != to && *to != PapyrusType::None && !(is_string_like(from) && is_string_like(to))
}

fn parse_ty(s: &str) -> PapyrusType {
    let s = s.trim();
    if let Some(elem) = s.strip_suffix("[]") {
        return PapyrusType::Array(Box::new(parse_ty(elem)));
    }
    match s.to_ascii_lowercase().as_str() {
        "int" => PapyrusType::Int,
        "float" => PapyrusType::Float,
        "bool" => PapyrusType::Bool,
        "string" => PapyrusType::String,
        "var" => PapyrusType::Var,
        "none" | "" => PapyrusType::None,
        _ => PapyrusType::Object(s.to_string()),
    }
}

fn arith_opcode(op: &str, result_ty: &PapyrusType) -> u8 {
    use PapyrusType::*;
    match (op, result_ty) {
        ("+", String) => OP_STRCAT,
        ("+", Float) => OP_FADD,
        ("+", _) => OP_IADD,
        ("-", Float) => OP_FSUB,
        ("-", _) => OP_ISUB,
        ("*", Float) => OP_FMUL,
        ("*", _) => OP_IMUL,
        ("/", Float) => OP_FDIV,
        ("/", _) => OP_IDIV,
        ("%", _) => OP_IMOD,
        _ => OP_IADD, // unreachable for Batch-1 operators
    }
}

/// User-flag table emitted into every FO4 `.pex`.
///
/// SLICE-HARDCODED: the six flags from `Institute_Papyrus_Flags.flg`
/// in one captured `.NET Hashtable` order. The order is non-reproducible across
/// exe runs; the golden comparison sorts user_flags so this is benign. A real
/// `.flg` parser is deferred.
fn default_user_flags(profile: GameProfile) -> Vec<PexUserFlagPayload> {
    if profile.game_id != 2 {
        return Vec::new();
    }
    [
        ("mandatory", 5u8),
        ("hidden", 0),
        ("default", 2),
        ("collapsedonref", 3),
        ("collapsedonbase", 4),
        ("conditional", 1),
    ]
    .into_iter()
    .map(|(name, index)| PexUserFlagPayload {
        name: name.to_string(),
        index,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pex::PexFilePayload;
    use crate::profile::Game;

    fn compile_src(src: &str) -> PexFilePayload {
        compile_src_with_source_name(src, None)
    }

    fn compile_src_with_source_name(src: &str, source_script_name: Option<&str>) -> PexFilePayload {
        let parsed = crate::parser::parse_script(src);
        let script_docstring = parsed.script_docstring.clone();
        let property_groups = parsed.property_groups.clone();
        let struct_names = parsed.struct_names.clone();
        let ast = parsed.ast.expect("parse");
        let resolver = SourceResolver::new(&[]);
        let profile = GameProfile::for_game(Game::Fo4);
        let tc = crate::typeck::typeck(&ast, &resolver, profile);
        compile(
            &ast,
            &tc,
            &resolver,
            profile,
            None,
            &script_docstring,
            &property_groups,
            &struct_names,
            source_script_name,
        )
    }

    /// Order-canonicalized view: zeroes §5 identity, sorts the non-deterministic
    /// regions (string table, user flags, per-state functions, debug functions)
    /// so a semantically-equal payload compares equal regardless of the exe's
    /// per-process `Hashtable` ordering.
    fn canon(p: &PexFilePayload) -> PexFilePayload {
        let mut p = p.clone();
        super::canonicalize(&mut p);
        p
    }

    /// Compile `src` and assert semantic parity with the stock-exe golden.
    fn check(src: &str, golden: &[u8]) {
        let mine = compile_src(src);
        let theirs = crate::pex::parse_pex_bytes(golden).expect("parse golden");
        assert_eq!(
            canon(&mine),
            canon(&theirs),
            "codegen diverges from the exe golden (semantic)"
        );
    }

    macro_rules! golden_test {
        ($name:ident, $src:expr, $file:literal) => {
            #[test]
            fn $name() {
                check(
                    $src,
                    include_bytes!(concat!("../tests/codegen_golden/", $file)),
                );
            }
        };
    }

    golden_test!(
        golden_int_add,
        "Scriptname GoldAdd\nInt Function F()\n  Return 1 + 2\nEndFunction\n",
        "GoldAdd.pex"
    );
    golden_test!(
        golden_return_literal,
        "Scriptname GRetLit\nInt Function F()\n  Return 5\nEndFunction\n",
        "GRetLit.pex"
    );
    golden_test!(
        golden_local,
        "Scriptname GLocal\nInt Function F()\n  Int x = 5\n  Return x\nEndFunction\n",
        "GLocal.pex"
    );
    golden_test!(
        golden_local_add,
        "Scriptname GLocalAdd\nInt Function F()\n  Int x = 1 + 2\n  Return x\nEndFunction\n",
        "GLocalAdd.pex"
    );
    golden_test!(
        golden_isub,
        "Scriptname GSub\nInt Function F()\n  Return 7 - 3\nEndFunction\n",
        "GSub.pex"
    );
    golden_test!(
        golden_imul,
        "Scriptname GMul\nInt Function F()\n  Return 2 * 3\nEndFunction\n",
        "GMul.pex"
    );
    golden_test!(
        golden_idiv,
        "Scriptname GDiv\nInt Function F()\n  Return 8 / 2\nEndFunction\n",
        "GDiv.pex"
    );
    golden_test!(
        golden_imod,
        "Scriptname GMod\nInt Function F()\n  Return 9 % 4\nEndFunction\n",
        "GMod.pex"
    );
    golden_test!(
        golden_fadd,
        "Scriptname GFloat\nFloat Function F()\n  Return 1.0 + 2.0\nEndFunction\n",
        "GFloat.pex"
    );
    golden_test!(
        golden_strcat,
        "Scriptname GStr\nString Function F()\n  Return \"a\" + \"b\"\nEndFunction\n",
        "GStr.pex"
    );
    golden_test!(
        golden_void_return,
        "Scriptname GVoid\nFunction F()\n  Return\nEndFunction\n",
        "GVoid.pex"
    );
    golden_test!(
        golden_empty_function,
        "Scriptname GEmpty\nFunction F()\nEndFunction\n",
        "GEmpty.pex"
    );
    golden_test!(
        golden_int_to_float_cast,
        "Scriptname GMix\nFloat Function F()\n  Return 1 + 2.0\nEndFunction\n",
        "GMix.pex"
    );
    golden_test!(
        golden_two_locals,
        "Scriptname GTwoLocal\nInt Function F()\n  Int x = 1 + 2\n  Int y = 3 + 4\n  Return x + y\nEndFunction\n",
        "GTwoLocal.pex"
    );
    golden_test!(
        golden_params,
        "Scriptname GParam\nInt Function F(Int a, Int b)\n  Return a + b\nEndFunction\n",
        "GParam.pex"
    );
    golden_test!(
        golden_bool_literal,
        "Scriptname GBool\nBool Function F()\n  Return True\nEndFunction\n",
        "GBool.pex"
    );
    golden_test!(
        golden_two_functions,
        "Scriptname GTwoFunc\nInt Function F()\n  Return 1 + 2\nEndFunction\nInt Function G()\n  Return 3 + 4\nEndFunction\n",
        "GTwoFunc.pex"
    );

    // --- comparisons, unary, cast, control flow, short-circuit ------
    golden_test!(
        golden_cmp_lt,
        "Scriptname GLt\nBool Function F()\n  Return 2 < 3\nEndFunction\n",
        "GLt.pex"
    );
    golden_test!(
        golden_cmp_le,
        "Scriptname GLe\nBool Function F()\n  Return 2 <= 3\nEndFunction\n",
        "GLe.pex"
    );
    golden_test!(
        golden_cmp_eq,
        "Scriptname GEq\nBool Function F()\n  Return 2 == 3\nEndFunction\n",
        "GEq.pex"
    );
    golden_test!(
        golden_cmp_ne,
        "Scriptname GNe\nBool Function F()\n  Return 2 != 3\nEndFunction\n",
        "GNe.pex"
    );
    golden_test!(
        golden_cmp_gt,
        "Scriptname GGt\nBool Function F()\n  Return 2 > 3\nEndFunction\n",
        "GGt.pex"
    );
    golden_test!(
        golden_cmp_ge,
        "Scriptname GGe\nBool Function F()\n  Return 2 >= 3\nEndFunction\n",
        "GGe.pex"
    );
    golden_test!(
        golden_not,
        "Scriptname GNot\nBool Function F(Bool a)\n  Return !a\nEndFunction\n",
        "GNot.pex"
    );
    golden_test!(
        golden_neg,
        "Scriptname GNeg\nInt Function F(Int a)\n  Return -a\nEndFunction\n",
        "GNeg.pex"
    );
    golden_test!(
        golden_as_cast,
        "Scriptname GCast\nInt Function F(Float a)\n  Return a as Int\nEndFunction\n",
        "GCast.pex"
    );
    golden_test!(
        golden_if,
        "Scriptname GIf\nInt Function F()\n  If 2 < 3\n    Return 1\n  EndIf\n  Return 0\nEndFunction\n",
        "GIf.pex"
    );
    golden_test!(
        golden_if_else,
        "Scriptname GIfElse\nInt Function F()\n  If 2 < 3\n    Return 1\n  Else\n    Return 0\n  EndIf\nEndFunction\n",
        "GIfElse.pex"
    );
    golden_test!(
        golden_if_elseif,
        "Scriptname GIfElseIf\nInt Function F(Int a)\n  If a == 1\n    Return 10\n  ElseIf a == 2\n    Return 20\n  Else\n    Return 0\n  EndIf\nEndFunction\n",
        "GIfElseIf.pex"
    );
    golden_test!(
        golden_while,
        "Scriptname GWhile\nInt Function F()\n  Int x = 0\n  While x < 3\n    x = x + 1\n  EndWhile\n  Return x\nEndFunction\n",
        "GWhile.pex"
    );
    golden_test!(
        golden_and,
        "Scriptname GAnd\nBool Function F(Bool a, Bool b)\n  Return a && b\nEndFunction\n",
        "GAnd.pex"
    );
    golden_test!(
        golden_or,
        "Scriptname GOr\nBool Function F(Bool a, Bool b)\n  Return a || b\nEndFunction\n",
        "GOr.pex"
    );

    // --- arrays, auto properties, same-script calls ----------------
    golden_test!(
        golden_arr_len,
        "Scriptname GArrLen\nInt Function F(Int[] a)\n  Return a.Length\nEndFunction\n",
        "GArrLen.pex"
    );
    golden_test!(
        golden_arr_get,
        "Scriptname GArrGet\nInt Function F(Int[] a)\n  Return a[0]\nEndFunction\n",
        "GArrGet.pex"
    );
    golden_test!(
        golden_arr_set,
        "Scriptname GArrSet\nFunction F(Int[] a)\n  a[0] = 5\nEndFunction\n",
        "GArrSet.pex"
    );
    golden_test!(
        golden_arr_new,
        "Scriptname GArrNew\nInt[] Function F()\n  Int[] a = new Int[4]\n  Return a\nEndFunction\n",
        "GArrNew.pex"
    );
    golden_test!(
        golden_arr_find,
        "Scriptname GArrFind\nInt Function F(Int[] a)\n  Return a.Find(7)\nEndFunction\n",
        "GArrFind.pex"
    );
    golden_test!(
        golden_arr_mut,
        "Scriptname GArrMut\nFunction F(Int[] a)\n  a.Add(7)\n  a.Remove(0)\n  a.Clear()\nEndFunction\n",
        "GArrMut.pex"
    );
    golden_test!(
        golden_struct,
        "Scriptname GStruct\nStruct Point\n  Int X\n  Int Y\nEndStruct\nInt Function F()\n  Point p = new Point\n  p.X = 5\n  Return p.Y\nEndFunction\n",
        "GStruct.pex"
    );
    #[test]
    fn namespaced_object_type_keeps_colons() {
        let p = compile_src(
            "Scriptname GNamespacedType\nQuests:_Default:ProgressBar:MasterScript ProgressBar\n",
        );
        assert_eq!(
            p.objects[0].variables[0].ty,
            "quests:_default:progressbar:masterscript"
        );
    }

    #[test]
    fn source_script_name_hint_sets_payload_name() {
        let p = compile_src_with_source_name(
            "Scriptname Foo:Bar\nFunction F()\nEndFunction\n",
            Some("foo:bar"),
        );
        assert_eq!(p.objects[0].name, "foo:bar");
        assert_eq!(
            p.debug_info.as_ref().unwrap().functions[0].object_name,
            "foo:bar"
        );
    }

    #[test]
    fn bare_primitive_locals_emit_default_assigns() {
        let p = compile_src(
            "Scriptname GLocalDefaults\nFunction F()\n  Int i\n  Float f\n  Bool b\n  String s\nEndFunction\n",
        );
        let f = &p.objects[0].states[0].functions[0];
        assert_eq!(f.instructions[0].opcode, OP_ASSIGN);
        assert_eq!(f.instructions[0].args, vec![ident("i"), int_value(0)]);
        assert_eq!(f.instructions[1].opcode, OP_ASSIGN);
        assert_eq!(
            f.instructions[1].args,
            vec![
                ident("f"),
                PexValuePayload {
                    value_type: VT_FLOAT,
                    data: serde_json::json!(0.0),
                },
            ]
        );
        assert_eq!(f.instructions[2].opcode, OP_ASSIGN);
        assert_eq!(
            f.instructions[2].args,
            vec![
                ident("b"),
                PexValuePayload {
                    value_type: VT_BOOL,
                    data: serde_json::json!(false),
                },
            ]
        );
        assert_eq!(f.instructions[3].opcode, OP_ASSIGN);
        assert_eq!(
            f.instructions[3].args,
            vec![
                ident("s"),
                PexValuePayload {
                    value_type: VT_STRING,
                    data: serde_json::json!(""),
                },
            ]
        );
    }

    #[test]
    fn missing_namespaced_static_call_uses_expected_assignment_type() {
        let p = compile_src(
            "Scriptname GMissingStatic\nFunction F()\n  GlobalVariable g = Foo:Bar.GetGlobal()\nEndFunction\n",
        );
        let f = &p.objects[0].states[0].functions[0];
        assert_eq!(f.instructions.len(), 2);
        assert_eq!(f.instructions[0].opcode, OP_CALLSTATIC);
        assert_eq!(f.instructions[0].args[0], ident("foo:bar"));
        assert_eq!(f.instructions[0].args[2], ident("::temp0"));
        assert_eq!(f.instructions[1].opcode, OP_ASSIGN);
        assert_eq!(f.instructions[1].args, vec![ident("g"), ident("::temp0")]);
        assert!(!f.instructions.iter().any(|i| i.opcode == OP_CAST));
    }

    #[test]
    fn full_property_literal_getter_returns_literal_without_cast() {
        let p = compile_src(
            "Scriptname GPropLit\nInt Property P\n  Int Function Get()\n    Return 5\n  EndFunction\nEndProperty\n",
        );
        let getter = p.objects[0].properties[0].getter.as_ref().unwrap();
        assert!(getter.locals.is_empty());
        assert_eq!(getter.instructions.len(), 1);
        assert_eq!(getter.instructions[0].opcode, OP_RETURN);
        assert_eq!(getter.instructions[0].args, vec![int_value(5)]);
    }

    #[test]
    fn call_argument_type_casing_does_not_cast() {
        let p = compile_src(
            "Scriptname GCallTypeCase\nform Property F Auto\nFunction Caller()\n  Callee(F)\nEndFunction\nFunction Callee(Form akForm)\nEndFunction\n",
        );
        let f = p.objects[0].states[0]
            .functions
            .iter()
            .find(|f| f.name == "Caller")
            .unwrap();
        assert!(!f.instructions.iter().any(|i| i.opcode == OP_CAST));
        assert_eq!(f.locals.len(), 1);
        assert_eq!(f.locals[0].name, "::nonevar");
    }

    #[test]
    fn local_argument_type_casing_still_casts() {
        let p = compile_src(
            "Scriptname GCallLocalTypeCase\nFunction Caller(Actor akActor)\n  Callee(akActor)\nEndFunction\nFunction Callee(actor akActor)\nEndFunction\n",
        );
        let f = p.objects[0].states[0]
            .functions
            .iter()
            .find(|f| f.name == "Caller")
            .unwrap();
        assert!(f.instructions.iter().any(|i| i.opcode == OP_CAST));
    }
    golden_test!(
        golden_self_call,
        "Scriptname GSelfCall\nInt Function F()\n  Return G()\nEndFunction\nInt Function G()\n  Return 1\nEndFunction\n",
        "GSelfCall.pex"
    );
    golden_test!(
        golden_global_call,
        "Scriptname GGlobalCall\nFunction F()\n  G()\nEndFunction\nFunction G() Global\nEndFunction\n",
        "GGlobalCall.pex"
    );
    golden_test!(
        golden_arg_call,
        "Scriptname GArgCall\nFunction F()\n  G(1, 2)\nEndFunction\nFunction G(Int a, Int b)\nEndFunction\n",
        "GArgCall.pex"
    );
    // Default-argument fill + arg binding: omitted (default materialized),
    // positional, and named-in-order (`abParam = true` / `b = true`).
    golden_test!(
        golden_default_args,
        "Scriptname GDefArg\nFunction Caller()\n  Helper()\n  Helper(true)\n  Helper(abParam = true)\n  Multi(1, b = true)\nEndFunction\nFunction Helper(bool abParam = false)\nEndFunction\nFunction Multi(int a, bool b = false)\nEndFunction\n",
        "GDefArg.pex"
    );
    golden_test!(
        golden_prop_get,
        "Scriptname GPropGet\nInt Property P Auto\nInt Function F()\n  Return P\nEndFunction\n",
        "GPropGet.pex"
    );
    golden_test!(
        golden_prop_set,
        "Scriptname GPropSet\nInt Property P Auto\nFunction F()\n  P = 5\nEndFunction\n",
        "GPropSet.pex"
    );

    // --- events, member variables, named states ----------------
    golden_test!(
        golden_event,
        "Scriptname GEvent extends Quest\nEvent OnInit()\n  Int x = 1\nEndEvent\n",
        "GEvent.pex"
    );
    golden_test!(
        golden_event_arg,
        "Scriptname GEventArg extends ObjectReference\nEvent OnActivate(ObjectReference akActionRef)\nEndEvent\n",
        "GEventArg.pex"
    );
    golden_test!(
        golden_var_set,
        "Scriptname GVarSet extends Quest\nInt _n\nFunction F()\n  _n = 5\nEndFunction\n",
        "GVarSet.pex"
    );
    golden_test!(
        golden_states,
        "Scriptname GStates extends Quest\nInt Function F()\n  Return 1\nEndFunction\nState Running\n  Int Function F()\n    Return 2\n  EndFunction\nEndState\n",
        "GStates.pex"
    );
    golden_test!(
        golden_prop_full,
        "Scriptname GPropFull extends Quest\nInt _p\nInt Property P\n  Int Function Get()\n    Return _p\n  EndFunction\n  Function Set(Int v)\n    _p = v\n  EndFunction\nEndProperty\n",
        "GPropFull.pex"
    );
    // Multi-property / multi-variable: the property and variable LISTS are
    // Hashtable-randomized, so this only passes because canon() sorts them.
    golden_test!(
        golden_multi,
        "Scriptname GMulti extends Quest\nInt _alpha\nFloat _beta\nBool _gamma\nInt Property Pone Auto\nFloat Property Ptwo Auto\nBool Property Pthree Auto\nString Property Pfour Auto\n",
        "GMulti.pex"
    );

    // --- slice regression (empty object stays byte-identical) ----------------

    #[test]
    fn slice_payload_matches_reference_structure() {
        let p = compile_src("ScriptName B21Slice extends Quest\n");
        assert_eq!((p.major_version, p.minor_version, p.game_id), (3, 9, 2));
        assert!(p.debug_info.is_none());
        assert_eq!(
            p.string_table,
            vec![
                "mandatory",
                "hidden",
                "default",
                "collapsedonref",
                "collapsedonbase",
                "conditional",
                "B21Slice",
                "Quest",
                "",
            ]
        );
        let o = &p.objects[0];
        assert_eq!((o.name.as_str(), o.parent.as_str()), ("B21Slice", "Quest"));
        assert_eq!(o.states.len(), 1);
        assert!(o.states[0].functions.is_empty());
    }

    #[test]
    fn slice_payload_round_trips_through_writer() {
        let p = compile_src("ScriptName B21Slice extends Quest\n");
        let bytes = crate::pex_writer::write_pex_bytes(&p).expect("write");
        let reparsed = crate::pex::parse_pex_bytes(&bytes).expect("reparse");
        assert_eq!(reparsed, p);
    }
}
