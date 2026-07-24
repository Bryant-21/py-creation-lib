/// Papyrus type checker — produces a `TypeTable` (node → type), a list of
/// implicit-cast sites, and diagnostics for type mismatches.
///
/// The pre-order walk order used here is the canonical one; `codegen.rs` uses
/// the identical order so `NodeId` indices line up between the two passes.
use crate::ast::*;
use crate::profile::GameProfile;
use crate::resolver::{Diagnostic, DiagnosticSeverity};
use crate::source_resolver::SourceResolver;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A resolved Papyrus type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PapyrusType {
    None,
    Bool,
    Int,
    Float,
    String,
    /// FO4/Starfield untyped "Var".
    Var,
    /// Script-object type, e.g. `ObjectReference`.
    Object(std::string::String),
    /// Array of element type.
    Array(Box<PapyrusType>),
    /// FO4/Starfield struct type.
    Struct(std::string::String),
}

impl std::fmt::Display for PapyrusType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PapyrusType::None => write!(f, "None"),
            PapyrusType::Bool => write!(f, "Bool"),
            PapyrusType::Int => write!(f, "Int"),
            PapyrusType::Float => write!(f, "Float"),
            PapyrusType::String => write!(f, "String"),
            PapyrusType::Var => write!(f, "Var"),
            PapyrusType::Object(n) => write!(f, "{n}"),
            PapyrusType::Array(elem) => write!(f, "{elem}[]"),
            PapyrusType::Struct(n) => write!(f, "{n}"),
        }
    }
}

/// Pre-order counter assigned once per visited AST node.  Codegen uses the
/// same walk so these indices are stable across passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

/// An implicit cast that the codegen must emit (no source-level `as`).
#[derive(Debug, Clone)]
pub struct CastSite {
    pub node: NodeId,
    pub from: PapyrusType,
    pub to: PapyrusType,
}

/// Flat map from node → resolved type, built during the walk.
#[derive(Debug, Default)]
pub struct TypeTable {
    map: HashMap<NodeId, PapyrusType>,
}

impl TypeTable {
    pub fn set(&mut self, id: NodeId, ty: PapyrusType) {
        self.map.insert(id, ty);
    }

    pub fn get(&self, id: NodeId) -> Option<&PapyrusType> {
        self.map.get(&id)
    }
}

/// Full output of one type-check pass.
#[derive(Debug)]
pub struct TypeckResult {
    pub types: TypeTable,
    pub casts: Vec<CastSite>,
    pub diagnostics: Vec<Diagnostic>,
    /// `(NodeId, fingerprint)` recorded in inference-recursion order. Used by
    /// the lockstep golden test to prove the recursion visits nodes in the same
    /// order the shared `compiler::walk_preorder` assigns ids.
    pub node_order: Vec<(NodeId, String)>,
}

#[cfg(test)]
impl TypeckResult {
    /// Test convenience: return the first table entry whose type equals `want`.
    pub fn type_of_first(&self, want: PapyrusType) -> Option<PapyrusType> {
        self.types.map.values().find(|t| **t == want).cloned()
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn typeck(ast: &ScriptNode, resolver: &SourceResolver, profile: GameProfile) -> TypeckResult {
    let mut tc = TypeChecker::new(ast, resolver, profile);
    tc.run();
    TypeckResult {
        types: tc.types,
        casts: tc.casts,
        diagnostics: tc.diagnostics,
        node_order: tc.node_order,
    }
}

// ---------------------------------------------------------------------------
// Assignability rules
// ---------------------------------------------------------------------------

/// Returns `true` when a value of type `from` may be implicitly used where
/// `to` is expected.  `is_subtype(child, parent)` answers whether `child`
/// extends `parent` transitively, via the import-path hierarchy walk at the
/// call site.
/// Papyrus type names are case-insensitive, so `ObjectReference[]` and
/// `objectReference[]` are the same type.
fn type_eq_ci(a: &PapyrusType, b: &PapyrusType) -> bool {
    use PapyrusType::*;
    match (a, b) {
        (Object(x), Object(y)) | (Struct(x), Struct(y)) => x.eq_ignore_ascii_case(y),
        (Array(x), Array(y)) => type_eq_ci(x, y),
        _ => a == b,
    }
}

/// A bare struct name `Name` and its qualified spelling `Owner:Name` denote the
/// same struct type (e.g. accessing `REScript`'s `DeadCount[]` member from
/// another script yields a value typed `DeadCount` that is assignable to a
/// `REScript:DeadCount` slot). Two differently-qualified names stay distinct.
fn struct_qual_eq(a: &str, b: &str) -> bool {
    let (aq, an) = a.rsplit_once(':').map_or((None, a), |(q, n)| (Some(q), n));
    let (bq, bn) = b.rsplit_once(':').map_or((None, b), |(q, n)| (Some(q), n));
    an.eq_ignore_ascii_case(bn)
        && match (aq, bq) {
            (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
            _ => true,
        }
}

pub fn is_implicitly_assignable(
    from: &PapyrusType,
    to: &PapyrusType,
    is_subtype: &impl Fn(&str, &str) -> bool,
) -> bool {
    use PapyrusType::*;
    if type_eq_ci(from, to) {
        return true;
    }
    match (from, to) {
        // Any type converts to String (concat / assignment).
        (_, String) => true,
        // Int widens to Float.
        (Int, Float) => true,
        // None assigns to any reference type.
        (None, Object(_)) | (None, Array(_)) | (None, Struct(_)) | (None, String) => true,
        // Up-cast via extends chain, or a bare/qualified spelling of one struct.
        (Object(c), Object(p)) => struct_qual_eq(c, p) || is_subtype(c, p),
        // Bool context: any type is truthy/falsy.
        (Int, Bool) | (Float, Bool) | (String, Bool) | (Object(_), Bool) => true,
        // Var accepts / produces anything (FO4 / Starfield).
        (Var, _) | (_, Var) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Internal checker
// ---------------------------------------------------------------------------

struct TypeChecker<'a> {
    ast: &'a ScriptNode,
    resolver: &'a SourceResolver,
    #[allow(dead_code)]
    profile: GameProfile,
    types: TypeTable,
    casts: Vec<CastSite>,
    diagnostics: Vec<Diagnostic>,
    /// NodeId for each id-bearing AST node, keyed by node address. Assigned once
    /// up front by `compiler::walk_preorder` so codegen reads the same ids.
    node_ids: HashMap<usize, NodeId>,
    /// `(NodeId, fingerprint)` in inference order — the lockstep witness.
    node_order: Vec<(NodeId, String)>,
    /// Script-level scope (properties + variables).
    script_scope: HashMap<std::string::String, PapyrusType>,
}

impl<'a> TypeChecker<'a> {
    fn new(ast: &'a ScriptNode, resolver: &'a SourceResolver, profile: GameProfile) -> Self {
        Self {
            ast,
            resolver,
            profile,
            types: TypeTable::default(),
            casts: Vec::new(),
            diagnostics: Vec::new(),
            node_ids: HashMap::new(),
            node_order: Vec::new(),
            script_scope: HashMap::new(),
        }
    }

    /// Look up the NodeId the shared walk assigned to `node`, recording it in
    /// inference order. Every node visited here is also visited by the walk.
    fn record(&mut self, node: crate::compiler::WalkNode<'_>) -> NodeId {
        let id = *self
            .node_ids
            .get(&crate::compiler::node_addr(&node))
            .expect("node must be assigned an id by the shared walk");
        self.node_order
            .push((id, crate::compiler::fingerprint(&node)));
        id
    }

    fn run(&mut self) {
        let ast = self.ast;

        // Single source of truth for NodeId assignment (shared with codegen).
        let mut ids: HashMap<usize, NodeId> = HashMap::new();
        crate::compiler::walk_preorder(ast, |id, node| {
            ids.insert(crate::compiler::node_addr(&node), id);
        });
        self.node_ids = ids;

        self.record(crate::compiler::WalkNode::Script(ast));
        self.build_script_scope();

        for f in &ast.functions {
            let scope = self.script_scope.clone();
            self.check_function(f, &scope);
        }
        for e in &ast.events {
            let scope = self.script_scope.clone();
            self.check_event(e, &scope);
        }
        for s in &ast.states {
            for f in &s.functions {
                let scope = self.script_scope.clone();
                self.check_function(f, &scope);
            }
            for e in &s.events {
                let scope = self.script_scope.clone();
                self.check_event(e, &scope);
            }
        }
    }

    fn build_script_scope(&mut self) {
        for p in &self.ast.properties {
            let ty = self.parse_type(&p.ty);
            self.script_scope.insert(p.name.to_ascii_lowercase(), ty);
        }
        for v in &self.ast.variables {
            let ty = self.parse_type(&v.ty);
            self.script_scope.insert(v.name.to_ascii_lowercase(), ty);
        }
    }

    /// Parse a Papyrus type string into `PapyrusType`.
    fn parse_type(&self, s: &str) -> PapyrusType {
        let s = s.trim();
        if s.ends_with("[]") {
            let elem = self.parse_type(&s[..s.len() - 2]);
            return PapyrusType::Array(Box::new(elem));
        }
        match s.to_ascii_lowercase().as_str() {
            "int" => PapyrusType::Int,
            "float" => PapyrusType::Float,
            "bool" => PapyrusType::Bool,
            "string" => PapyrusType::String,
            "var" => PapyrusType::Var,
            "none" | "" => PapyrusType::None,
            _ => PapyrusType::Object(s.to_owned()),
        }
    }

    /// Type of a same-script struct member, if `type_name` (bare `Struct` or
    /// `thisscript#struct`) names a struct declared in this script.
    fn struct_member_ty(&self, type_name: &str, member: &str) -> Option<PapyrusType> {
        let base = type_name.trim_end_matches("[]");
        let sname = match base.split_once('#') {
            Some((sc, sn)) if sc.eq_ignore_ascii_case(&self.ast.name) => sn,
            Some(_) => return None,
            None => base,
        };
        let st = self
            .ast
            .structs
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(sname))?;
        st.members
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(member))
            .map(|m| self.parse_type(&m.ty))
    }

    fn check_function(
        &mut self,
        func: &FunctionDef,
        scope: &HashMap<std::string::String, PapyrusType>,
    ) {
        self.record(crate::compiler::WalkNode::Function(func));
        let ret_ty = self.parse_type(&func.return_type);
        let mut local = scope.clone();
        for p in &func.params {
            let ty = self.parse_type(&p.ty);
            local.insert(p.name.to_ascii_lowercase(), ty);
        }
        self.check_body(&func.body, &mut local, &ret_ty);
    }

    fn check_event(&mut self, ev: &EventDef, scope: &HashMap<std::string::String, PapyrusType>) {
        self.record(crate::compiler::WalkNode::Event(ev));
        self.check_inherited_event_signature(ev);
        let mut local = scope.clone();
        for p in &ev.params {
            let ty = self.parse_type(&p.ty);
            local.insert(p.name.to_ascii_lowercase(), ty);
        }
        // Events return None implicitly.
        self.check_body(&ev.body, &mut local, &PapyrusType::None);
    }

    fn check_inherited_event_signature(&mut self, ev: &EventDef) {
        let Some(parent) = self.ast.parent.as_deref() else {
            return;
        };
        let Some((declaring_script, inherited)) = self.resolver.get_event(parent, &ev.name) else {
            return;
        };
        let matches = ev.params.len() == inherited.params.len()
            && ev
                .params
                .iter()
                .zip(&inherited.params)
                .all(|(actual, expected)| {
                    type_eq_ci(&self.parse_type(&actual.ty), &self.parse_type(&expected.ty))
                });
        if matches {
            return;
        }

        let signature = |event: &EventDef| {
            event
                .params
                .iter()
                .map(|param| param.ty.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.diagnostics.push(Diagnostic {
            line: ev.pos.line,
            col: ev.pos.col,
            end_line: ev.pos.end_line,
            end_col: ev.pos.end_col,
            message: format!(
                "event '{}' does not match inherited signature from '{}': expected ({}) but found ({})",
                ev.name,
                declaring_script,
                signature(&inherited),
                signature(ev),
            ),
            severity: DiagnosticSeverity::Error,
        });
    }

    fn check_body(
        &mut self,
        stmts: &[Stmt],
        scope: &mut HashMap<std::string::String, PapyrusType>,
        ret_ty: &PapyrusType,
    ) {
        for s in stmts {
            self.check_stmt(s, scope, ret_ty);
        }
    }

    fn check_stmt(
        &mut self,
        stmt: &Stmt,
        scope: &mut HashMap<std::string::String, PapyrusType>,
        ret_ty: &PapyrusType,
    ) {
        match stmt {
            Stmt::ExprStmt { expr, .. } => {
                self.visit_expr(expr, scope);
            }
            Stmt::AssignStmt {
                target,
                op,
                value,
                pos,
            } => {
                let (target_id, target_ty) = self.visit_expr(target, scope);
                let (val_id, val_ty) = self.visit_expr(value, scope);
                if op == "=" {
                    self.check_assignable(&val_ty, &target_ty, val_id, *pos);
                } else {
                    let arithmetic_op = op.strip_suffix('=').unwrap_or(op);
                    let result_ty = self.binary_result_type(
                        arithmetic_op,
                        target_id,
                        &target_ty,
                        val_id,
                        &val_ty,
                        *pos,
                    );
                    self.check_assignable(&result_ty, &target_ty, target_id, *pos);
                }
            }
            Stmt::ReturnStmt {
                value: Some(v),
                pos,
            } => {
                let (val_id, val_ty) = self.visit_expr(v, scope);
                self.check_assignable(&val_ty, ret_ty, val_id, *pos);
            }
            Stmt::ReturnStmt { value: None, .. } => {}
            Stmt::IfStmt {
                condition,
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                self.visit_expr(condition, scope);
                self.check_body(body, scope, ret_ty);
                for c in elseif_clauses {
                    self.visit_expr(&c.condition, scope);
                    self.check_body(&c.body, scope, ret_ty);
                }
                self.check_body(else_body, scope, ret_ty);
            }
            Stmt::WhileStmt {
                condition, body, ..
            } => {
                self.visit_expr(condition, scope);
                self.check_body(body, scope, ret_ty);
            }
            Stmt::LocalVarStmt {
                name,
                ty,
                value,
                pos,
            } => {
                let decl_ty = self.parse_type(ty);
                scope.insert(name.to_ascii_lowercase(), decl_ty.clone());
                if let Some(init) = value {
                    let (init_id, init_ty) = self.visit_expr(init, scope);
                    self.check_assignable(&init_ty, &decl_ty, init_id, *pos);
                }
            }
        }
    }

    /// Visit an expression, assign it a `NodeId` and infer its type.
    fn visit_expr(
        &mut self,
        expr: &Expr,
        scope: &HashMap<std::string::String, PapyrusType>,
    ) -> (NodeId, PapyrusType) {
        let id = self.record(crate::compiler::WalkNode::Expr(expr));
        let ty = self.infer_expr(expr, scope);
        self.types.set(id, ty.clone());
        (id, ty)
    }

    fn infer_expr(
        &mut self,
        expr: &Expr,
        scope: &HashMap<std::string::String, PapyrusType>,
    ) -> PapyrusType {
        match expr {
            Expr::LiteralExpr { ty, value, .. } => match ty.as_str() {
                "int" => PapyrusType::Int,
                "float" => PapyrusType::Float,
                "bool" => PapyrusType::Bool,
                "string" => PapyrusType::String,
                "none" => PapyrusType::None,
                _ => match value {
                    LiteralValue::Null => PapyrusType::None,
                    LiteralValue::Bool(_) => PapyrusType::Bool,
                    LiteralValue::Int(_) => PapyrusType::Int,
                    LiteralValue::Float(_) => PapyrusType::Float,
                    LiteralValue::Str(_) => PapyrusType::String,
                },
            },

            Expr::NameExpr { name, .. } => {
                let lower = name.to_ascii_lowercase();
                if lower == "self" {
                    return PapyrusType::Object(self.ast.name.clone());
                }
                if lower == "parent" {
                    return self
                        .ast
                        .parent
                        .as_deref()
                        .map(|p| PapyrusType::Object(p.to_owned()))
                        .unwrap_or(PapyrusType::None);
                }
                scope.get(&lower).cloned().unwrap_or(PapyrusType::None)
            }

            Expr::ParentExpr { .. } => self
                .ast
                .parent
                .as_deref()
                .map(|p| PapyrusType::Object(p.to_owned()))
                .unwrap_or(PapyrusType::None),

            Expr::CastExpr {
                expr: inner,
                target_type,
                ..
            } => {
                self.visit_expr(inner, scope);
                self.parse_type(target_type)
            }

            Expr::BinaryExpr {
                left,
                op,
                right,
                pos,
            } => {
                let (left_id, left_ty) = self.visit_expr(left, scope);
                let (right_id, right_ty) = self.visit_expr(right, scope);
                self.binary_result_type(op, left_id, &left_ty, right_id, &right_ty, *pos)
            }

            Expr::UnaryExpr { op, operand, .. } => {
                let (_oid, oty) = self.visit_expr(operand, scope);
                match op.as_str() {
                    "!" => PapyrusType::Bool,
                    _ => oty,
                }
            }

            Expr::CallExpr { args, function, .. } => {
                for a in args {
                    self.visit_expr(a, scope);
                }
                let imports: Vec<String> = self
                    .ast
                    .imports
                    .iter()
                    .map(|i| i.script_name.clone())
                    .collect();
                self.resolver
                    .get_function_return_type(&self.ast.name, function)
                    .or_else(|| {
                        self.resolver
                            .find_imported_global(&imports, function)
                            .map(|(_, f)| f.return_type)
                    })
                    .map(|t| self.parse_type(&t))
                    .unwrap_or(PapyrusType::None)
            }

            Expr::DotCallExpr {
                object,
                method,
                args,
                ..
            } => {
                let (_oid, obj_ty) = self.visit_expr(object, scope);
                for a in args {
                    self.visit_expr(a, scope);
                }
                // Array built-in methods: the search methods return Int; the
                // mutators (Add/Insert/Remove/RemoveLast/Clear) are void.
                if matches!(obj_ty, PapyrusType::Array(_)) {
                    match method.to_ascii_lowercase().as_str() {
                        "find" | "rfind" | "findstruct" | "rfindstruct" => return PapyrusType::Int,
                        "add" | "insert" | "remove" | "removelast" | "clear" => {
                            return PapyrusType::None;
                        }
                        _ => {}
                    }
                }
                let script_name = match &obj_ty {
                    PapyrusType::Object(n) => n.clone(),
                    // Receiver may be a script TYPE name calling a global function
                    // on it (e.g. `Utility.RandomFloat()`, `Math.Cos()`).
                    _ => match &**object {
                        Expr::NameExpr { name, .. } if self.resolver.script_exists(name) => {
                            name.clone()
                        }
                        _ => std::string::String::new(),
                    },
                };
                self.resolver
                    .get_function_return_type(&script_name, method)
                    .map(|t| self.parse_type(&t))
                    .unwrap_or(PapyrusType::None)
            }

            Expr::DotExpr { object, member, .. } => {
                let (_oid, obj_ty) = self.visit_expr(object, scope);
                // Array pseudo-property `.Length` is Int.
                if matches!(obj_ty, PapyrusType::Array(_)) && member.eq_ignore_ascii_case("length")
                {
                    return PapyrusType::Int;
                }
                // Same-script struct member access.
                if let Some(t) = self.struct_member_ty(&obj_ty.to_string(), member) {
                    return t;
                }
                let script_name = match &obj_ty {
                    PapyrusType::Object(n) => n.clone(),
                    _ => std::string::String::new(),
                };
                self.resolver
                    .get_properties(&script_name)
                    .into_iter()
                    .find(|p| p.name.eq_ignore_ascii_case(member))
                    .map(|p| self.parse_type(&p.ty))
                    .unwrap_or(PapyrusType::None)
            }

            Expr::ArrayAccessExpr { array, index, .. } => {
                let (_aid, arr_ty) = self.visit_expr(array, scope);
                self.visit_expr(index, scope);
                match arr_ty {
                    PapyrusType::Array(elem) => *elem,
                    _ => PapyrusType::None,
                }
            }

            Expr::NewArrayExpr {
                element_type, size, ..
            } => {
                self.visit_expr(size, scope);
                let elem = self.parse_type(element_type);
                PapyrusType::Array(Box::new(elem))
            }
        }
    }

    /// Determine the result type of a binary operation and record cast sites.
    fn binary_result_type(
        &mut self,
        op: &str,
        left_id: NodeId,
        left_ty: &PapyrusType,
        right_id: NodeId,
        right_ty: &PapyrusType,
        _pos: Pos,
    ) -> PapyrusType {
        let result_ty = binary_result_type(op, left_ty, right_ty);

        if op == "+" && result_ty == PapyrusType::String {
            if left_ty != &PapyrusType::String {
                self.casts.push(CastSite {
                    node: left_id,
                    from: left_ty.clone(),
                    to: PapyrusType::String,
                });
            }
            if right_ty != &PapyrusType::String {
                self.casts.push(CastSite {
                    node: right_id,
                    from: right_ty.clone(),
                    to: PapyrusType::String,
                });
            }
        }

        result_ty
    }

    /// Check that `from` is assignable to `to`; push a cast site or an error.
    fn check_assignable(&mut self, from: &PapyrusType, to: &PapyrusType, node: NodeId, pos: Pos) {
        if to == &PapyrusType::None {
            // Void return type — nothing to check.
            return;
        }
        let resolver = self.resolver;
        let ok = is_implicitly_assignable(from, to, &|child, parent| {
            resolver
                .get_hierarchy(child)
                .iter()
                .any(|s| s.eq_ignore_ascii_case(parent))
        });
        if ok {
            if from != to {
                self.casts.push(CastSite {
                    node,
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        } else {
            self.diagnostics.push(Diagnostic {
                line: pos.line,
                col: pos.col,
                end_line: pos.end_line,
                end_col: pos.end_col,
                message: format!("cannot assign {from} to {to}"),
                severity: DiagnosticSeverity::Error,
            });
        }
    }
}

pub(crate) fn binary_result_type(
    op: &str,
    left_ty: &PapyrusType,
    right_ty: &PapyrusType,
) -> PapyrusType {
    use PapyrusType::*;

    if op == "+" && (matches!(left_ty, String) || matches!(right_ty, String)) {
        return String;
    }
    if matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=" | "&&" | "||") {
        return Bool;
    }
    match (left_ty, right_ty) {
        (Float, _) | (_, Float) => Float,
        (Int, Int) => Int,
        _ => left_ty.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn typeck_source(src: &str) -> TypeckResult {
        let parsed = crate::parser::parse_script(src);
        assert!(
            parsed.errors.is_empty(),
            "parse errors: {:?}",
            parsed.errors
        );
        let ast = parsed.ast.expect("expected AST");
        let resolver = crate::source_resolver::SourceResolver::new(&[]);
        typeck(
            &ast,
            &resolver,
            crate::profile::GameProfile::for_game(crate::profile::Game::Fo4),
        )
    }

    #[cfg(test)]
    fn empty_hierarchy() -> impl Fn(&str, &str) -> bool {
        |_, _| false
    }

    #[test]
    fn assigns_literal_types() {
        let r = typeck_source(
            "Scriptname T\nInt Function F()\n  Int x = 1 + 2\n  Return x\nEndFunction\n",
        );
        assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
        assert_eq!(r.type_of_first(PapyrusType::Int), Some(PapyrusType::Int));
    }

    #[test]
    fn profile_feature_flags() {
        use crate::profile::{Game, GameProfile};
        let skyrim = GameProfile::for_game(Game::SkyrimSe);
        assert!(!skyrim.allow_structs);
        assert!(!skyrim.allow_groups);
        assert!(!skyrim.allow_guards);

        let fo4 = GameProfile::for_game(Game::Fo4);
        assert!(fo4.allow_structs);
        assert!(fo4.allow_groups);
        assert!(!fo4.allow_guards);

        let sf = GameProfile::for_game(Game::Starfield);
        assert!(sf.allow_structs);
        assert!(sf.allow_groups);
        assert!(sf.allow_guards);
    }

    #[test]
    fn implicit_cast_table() {
        use PapyrusType::*;
        let cases = [
            (Int, Float, true),
            (Int, String, true),
            (Float, Int, false),
            (None, Object("ObjectReference".into()), true),
            (Object("Actor".into()), String, true),
            (String, Int, false),
        ];
        for (from, to, expected) in cases {
            assert_eq!(
                is_implicitly_assignable(&from, &to, &empty_hierarchy()),
                expected,
                "{from:?} -> {to:?}"
            );
        }
    }

    #[test]
    fn rejects_type_errors() {
        let bad = [
            (
                // String returned where Int expected.
                "Scriptname T\nInt Function F()\n  String s = \"x\"\n  Return s\nEndFunction\n",
                "cannot",
            ),
            (
                // String assigned to Int local.
                "Scriptname T\nFunction F()\n  Int x = \"hello\"\nEndFunction\n",
                "cannot",
            ),
        ];
        for (src, needle) in bad {
            let r = typeck_source(src);
            assert!(
                r.diagnostics.iter().any(|d| d.message.contains(needle)),
                "expected error containing {needle:?} for {src:?}, got {:?}",
                r.diagnostics
            );
        }
    }

    #[test]
    fn compound_assignments_typecheck_the_arithmetic_result() {
        let valid = typeck_source(
            "Scriptname TCompoundValid\nFunction F()\n  Float f = 1.0\n  Int i = 1\n  Bool b = True\n  String s = \"x\"\n  f += i\n  i += b\n  s += i\nEndFunction\n",
        );
        assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);

        let invalid = typeck_source(
            "Scriptname TCompoundInvalid\nFunction F()\n  Int i = 1\n  Float f = 1.0\n  i += f\nEndFunction\n",
        );
        assert!(
            invalid
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cannot assign Float to Int")),
            "{:?}",
            invalid.diagnostics
        );
    }
}
