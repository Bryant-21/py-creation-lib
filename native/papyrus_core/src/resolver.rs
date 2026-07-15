//! Symbol resolver — walks an AST and emits diagnostics for unresolved references.
//!
//! Mirror of `py_creation_lib/python/creation_lib/papyrus_lsp/resolver.py`. Pure-Rust: no PyO3, no I/O beyond
//! the `ScriptDB` it borrows. The resolver does *not* infer Papyrus types
//! beyond what the script DB tells it — same scope as the Python original.

use crate::ast::*;
use crate::script_db::ScriptDB;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Serialize)]
#[repr(u8)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

pub fn resolve(ast: &ScriptNode, db: &ScriptDB) -> Vec<Diagnostic> {
    let mut r = Resolver::new(ast, db);
    r.run();
    r.diagnostics
}

struct Resolver<'a> {
    ast: &'a ScriptNode,
    db: &'a ScriptDB,
    diagnostics: Vec<Diagnostic>,
    /// Lower-cased name → declared type. Built from properties + variables; per
    /// function/event we layer params on top in a clone.
    scope: HashMap<String, String>,
}

impl<'a> Resolver<'a> {
    fn new(ast: &'a ScriptNode, db: &'a ScriptDB) -> Self {
        Self {
            ast,
            db,
            diagnostics: Vec::new(),
            scope: HashMap::new(),
        }
    }

    fn run(&mut self) {
        self.check_extends();
        self.check_imports();
        self.build_script_scope();

        // We can't borrow `self.ast` while calling `&mut self` methods, so
        // temporarily take ownership of the parts we need to walk.
        let funcs = self.ast.functions.clone();
        for f in &funcs {
            self.resolve_function(f);
        }
        let events = self.ast.events.clone();
        for e in &events {
            self.resolve_event(e);
        }
        let states = self.ast.states.clone();
        for s in &states {
            for f in &s.functions {
                self.resolve_function(f);
            }
            for e in &s.events {
                self.resolve_event(e);
            }
        }
    }

    fn check_extends(&mut self) {
        if let Some(parent) = &self.ast.parent {
            if !self.db.script_exists(parent) {
                let pos = self.ast.pos;
                self.diagnostics.push(Diagnostic {
                    line: pos.line,
                    col: pos.col,
                    end_line: pos.line,
                    end_col: pos.col + parent.len() as u32,
                    message: format!("Unknown parent script '{parent}'"),
                    severity: DiagnosticSeverity::Error,
                });
            }
        }
    }

    fn check_imports(&mut self) {
        for imp in &self.ast.imports {
            if !self.db.script_exists(&imp.script_name) {
                let pos = imp.pos;
                self.diagnostics.push(Diagnostic {
                    line: pos.line,
                    col: pos.col,
                    end_line: pos.end_line,
                    end_col: pos.end_col,
                    message: format!("Unknown imported script '{}'", imp.script_name),
                    severity: DiagnosticSeverity::Error,
                });
            }
        }
    }

    fn build_script_scope(&mut self) {
        for p in &self.ast.properties {
            self.scope.insert(p.name.to_ascii_lowercase(), p.ty.clone());
        }
        for v in &self.ast.variables {
            self.scope.insert(v.name.to_ascii_lowercase(), v.ty.clone());
        }
    }

    fn resolve_function(&mut self, func: &FunctionDef) {
        let mut local = self.scope.clone();
        for p in &func.params {
            local.insert(p.name.to_ascii_lowercase(), p.ty.clone());
        }
        self.resolve_body(&func.body, &mut local);
    }

    fn resolve_event(&mut self, ev: &EventDef) {
        let mut local = self.scope.clone();
        for p in &ev.params {
            local.insert(p.name.to_ascii_lowercase(), p.ty.clone());
        }
        self.resolve_body(&ev.body, &mut local);
    }

    fn resolve_body(&mut self, stmts: &[Stmt], scope: &mut HashMap<String, String>) {
        for s in stmts {
            self.resolve_stmt(s, scope);
        }
    }

    fn resolve_stmt(&mut self, stmt: &Stmt, scope: &mut HashMap<String, String>) {
        match stmt {
            Stmt::ExprStmt { expr, .. } => self.resolve_expr(expr, scope),
            Stmt::AssignStmt { target, value, .. } => {
                self.resolve_expr(target, scope);
                self.resolve_expr(value, scope);
            }
            Stmt::ReturnStmt { value: Some(v), .. } => self.resolve_expr(v, scope),
            Stmt::ReturnStmt { value: None, .. } => {}
            Stmt::IfStmt {
                condition,
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                self.resolve_expr(condition, scope);
                self.resolve_body(body, scope);
                for c in elseif_clauses {
                    self.resolve_expr(&c.condition, scope);
                    self.resolve_body(&c.body, scope);
                }
                self.resolve_body(else_body, scope);
            }
            Stmt::WhileStmt {
                condition, body, ..
            } => {
                self.resolve_expr(condition, scope);
                self.resolve_body(body, scope);
            }
            Stmt::LocalVarStmt {
                name, ty, value, ..
            } => {
                scope.insert(name.to_ascii_lowercase(), ty.clone());
                if let Some(v) = value {
                    self.resolve_expr(v, scope);
                }
            }
        }
    }

    fn resolve_expr(&mut self, expr: &Expr, scope: &HashMap<String, String>) {
        match expr {
            Expr::DotCallExpr {
                object,
                method,
                args,
                pos,
            } => {
                let orig = self.infer_type(object, scope);
                let receiver = self.resolve_receiver_type(object, scope);
                if let Some(rt) = receiver {
                    let is_self = orig
                        .as_deref()
                        .map(|t| self.is_self_type(t))
                        .unwrap_or(false);
                    if !(is_self && self.ast_has_function(method))
                        && !self.db.has_function(&rt, method)
                    {
                        self.diagnostics.push(Diagnostic {
                            line: pos.line,
                            col: pos.col,
                            end_line: pos.end_line,
                            end_col: pos.end_col,
                            message: format!("'{rt}' has no function '{method}'"),
                            severity: DiagnosticSeverity::Error,
                        });
                    }
                }
                for a in args {
                    self.resolve_expr(a, scope);
                }
            }
            Expr::DotExpr {
                object,
                member,
                pos,
            } => {
                let orig = self.infer_type(object, scope);
                let receiver = self.resolve_receiver_type(object, scope);
                if let Some(rt) = receiver {
                    let is_self = orig
                        .as_deref()
                        .map(|t| self.is_self_type(t))
                        .unwrap_or(false);
                    if !(is_self && self.ast_has_property(member))
                        && !self.db.has_property(&rt, member)
                        && !member.eq_ignore_ascii_case("length")
                    {
                        self.diagnostics.push(Diagnostic {
                            line: pos.line,
                            col: pos.col,
                            end_line: pos.end_line,
                            end_col: pos.end_col,
                            message: format!("'{rt}' has no property '{member}'"),
                            severity: DiagnosticSeverity::Warning,
                        });
                    }
                }
            }
            Expr::CallExpr { args, .. } => {
                for a in args {
                    self.resolve_expr(a, scope);
                }
            }
            Expr::BinaryExpr { left, right, .. } => {
                self.resolve_expr(left, scope);
                self.resolve_expr(right, scope);
            }
            Expr::UnaryExpr { operand, .. } => self.resolve_expr(operand, scope),
            Expr::CastExpr { expr, .. } => self.resolve_expr(expr, scope),
            Expr::ArrayAccessExpr { array, index, .. } => {
                self.resolve_expr(array, scope);
                self.resolve_expr(index, scope);
            }
            Expr::NewArrayExpr { size, .. } => self.resolve_expr(size, scope),
            Expr::NameExpr { .. } | Expr::LiteralExpr { .. } | Expr::ParentExpr { .. } => {}
        }
    }

    fn resolve_receiver_type(
        &self,
        expr: &Expr,
        scope: &HashMap<String, String>,
    ) -> Option<String> {
        let receiver = self.infer_type(expr, scope)?;
        if !self.db.script_exists(&receiver) {
            if self.is_self_type(&receiver) {
                return self.ast.parent.clone();
            }
            return None;
        }
        Some(receiver)
    }

    fn infer_type(&self, expr: &Expr, scope: &HashMap<String, String>) -> Option<String> {
        match expr {
            Expr::NameExpr { name, .. } => {
                let l = name.to_ascii_lowercase();
                if l == "self" {
                    return Some(self.ast.name.clone());
                }
                if l == "parent" {
                    return self.ast.parent.clone();
                }
                if let Some(t) = scope.get(&l) {
                    return Some(t.clone());
                }
                if self.db.script_exists(name) {
                    return Some(name.clone());
                }
                None
            }
            Expr::ParentExpr { .. } => self.ast.parent.clone(),
            Expr::DotCallExpr { object, method, .. } => {
                let receiver = self.infer_type(object, scope)?;
                self.db.get_function_return_type(&receiver, method)
            }
            Expr::CastExpr { target_type, .. } => Some(target_type.clone()),
            _ => None,
        }
    }

    fn is_self_type(&self, type_name: &str) -> bool {
        let tl = type_name.to_ascii_lowercase();
        let short = self
            .ast
            .name
            .rsplit(':')
            .next()
            .unwrap_or(&self.ast.name)
            .to_ascii_lowercase();
        tl == short || tl == self.ast.name.to_ascii_lowercase()
    }

    fn ast_has_function(&self, name: &str) -> bool {
        let l = name.to_ascii_lowercase();
        self.ast
            .functions
            .iter()
            .any(|f| f.name.to_ascii_lowercase() == l)
            || self
                .ast
                .states
                .iter()
                .flat_map(|s| s.functions.iter())
                .any(|f| f.name.to_ascii_lowercase() == l)
    }

    fn ast_has_property(&self, name: &str) -> bool {
        let l = name.to_ascii_lowercase();
        self.ast
            .properties
            .iter()
            .any(|p| p.name.to_ascii_lowercase() == l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_script;

    fn ast_of(src: &str) -> ScriptNode {
        let r = parse_script(src);
        if !r.errors.is_empty() {
            panic!("parser errors: {:?}", r.errors);
        }
        r.ast.expect("expected AST")
    }

    #[test]
    fn unknown_parent_emits_diagnostic() {
        let ast = ast_of("ScriptName Foo extends DoesNotExist\n");
        let db = ScriptDB::empty();
        let diags = resolve(&ast, &db);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("DoesNotExist"));
    }

    #[test]
    fn self_property_no_diagnostic() {
        let ast = ast_of(
            "ScriptName Foo\n\
             Int Property X Auto\n\
             Function Bar()\n\
               Int y = Self.X\n\
             EndFunction\n",
        );
        let db = ScriptDB::empty();
        let diags = resolve(&ast, &db);
        // Self.X should resolve via ast_has_property — no diagnostic on .X.
        let on_x: Vec<_> = diags.iter().filter(|d| d.message.contains("'X'")).collect();
        assert!(on_x.is_empty(), "unexpected: {:?}", diags);
    }

    #[test]
    fn local_var_added_to_scope() {
        let ast = ast_of(
            "ScriptName Foo\n\
             Function Bar()\n\
               Int x = 5\n\
               Int y = x + 1\n\
             EndFunction\n",
        );
        let db = ScriptDB::empty();
        let diags = resolve(&ast, &db);
        // No DB to validate types against; we just want this to not panic and
        // produce no spurious diagnostics for `x`.
        assert!(diags.iter().all(|d| !d.message.contains("'x'")));
    }

    #[test]
    fn registered_self_script_resolves_dotcall() {
        let ast = ast_of(
            "ScriptName Foo extends Bar\n\
             Function DoIt()\n\
             EndFunction\n\
             Function Wrapper()\n\
               Self.DoIt()\n\
             EndFunction\n",
        );
        let db = ScriptDB::empty();
        db.register_ast(&ast);
        let diags = resolve(&ast, &db);
        assert!(diags.iter().all(|d| !d.message.contains("DoIt")));
    }
}
