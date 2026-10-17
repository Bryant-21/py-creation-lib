//! Compiler orchestration + the single canonical AST walk.
//!
//! `walk_preorder` is the one deterministic pre-order NodeId assignment shared
//! by `typeck` (keys its `TypeTable` by NodeId) and `codegen` (looks types up by
//! NodeId), which keeps the two in lockstep. See `typeck_and_codegen_share_node_ids`.
//!
//! `compile_source` is the end-to-end entry point: parse → typeck → codegen →
//! neutralize identity fields → write bytes.

use crate::ast::*;
use crate::profile::{Game, GameProfile};
use crate::resolver::{Diagnostic, DiagnosticSeverity};
use crate::typeck::NodeId;
use std::path::{Component, Path};

// ---------------------------------------------------------------------------
// Canonical NodeId walk
// ---------------------------------------------------------------------------

/// One AST node that receives a `NodeId`. Only the script, its functions and
/// events (including those nested in states), and every expression are
/// id-bearing — statements are not (mirrors `typeck`).
#[derive(Clone, Copy)]
pub enum WalkNode<'a> {
    Script(&'a ScriptNode),
    Function(&'a FunctionDef),
    Event(&'a EventDef),
    Expr(&'a Expr),
}

/// Drive `visit` once per id-bearing node in canonical pre-order, handing it the
/// `NodeId` that node receives. Single source of truth for id assignment.
pub fn walk_preorder<'a>(ast: &'a ScriptNode, mut visit: impl FnMut(NodeId, WalkNode<'a>)) {
    let mut w = Walker {
        next: 0,
        visit: &mut visit,
    };
    w.script(ast);
}

/// Stable address of the node a `WalkNode` points at, used as the lookup key
/// shared between the walk and `typeck`'s inference recursion.
pub fn node_addr(n: &WalkNode<'_>) -> usize {
    match n {
        WalkNode::Script(s) => *s as *const ScriptNode as usize,
        WalkNode::Function(f) => *f as *const FunctionDef as usize,
        WalkNode::Event(e) => *e as *const EventDef as usize,
        WalkNode::Expr(e) => *e as *const Expr as usize,
    }
}

/// Structural fingerprint used only by the lockstep golden test.
pub fn fingerprint(n: &WalkNode<'_>) -> String {
    match n {
        WalkNode::Script(s) => format!("script:{}", s.name),
        WalkNode::Function(f) => format!("fn:{}", f.name),
        WalkNode::Event(e) => format!("event:{}", e.name),
        WalkNode::Expr(e) => format!("expr:{}", expr_tag(e)),
    }
}

fn expr_tag(e: &Expr) -> String {
    match e {
        Expr::NameExpr { name, .. } => format!("name:{name}"),
        Expr::LiteralExpr { ty, .. } => format!("lit:{ty}"),
        Expr::DotExpr { member, .. } => format!("dot:{member}"),
        Expr::CallExpr { function, .. } => format!("call:{function}"),
        Expr::DotCallExpr { method, .. } => format!("dotcall:{method}"),
        Expr::BinaryExpr { op, .. } => format!("bin:{op}"),
        Expr::UnaryExpr { op, .. } => format!("un:{op}"),
        Expr::CastExpr { target_type, .. } => format!("cast:{target_type}"),
        Expr::ArrayAccessExpr { .. } => "arrindex".to_string(),
        Expr::NewArrayExpr { element_type, .. } => format!("newarr:{element_type}"),
        Expr::ParentExpr { .. } => "parent".to_string(),
    }
}

struct Walker<'a, 'f> {
    next: u32,
    visit: &'f mut dyn FnMut(NodeId, WalkNode<'a>),
}

impl<'a> Walker<'a, '_> {
    fn alloc(&mut self, node: WalkNode<'a>) {
        let id = NodeId(self.next);
        self.next += 1;
        (self.visit)(id, node);
    }

    fn script(&mut self, s: &'a ScriptNode) {
        self.alloc(WalkNode::Script(s));
        for f in &s.functions {
            self.function(f);
        }
        for e in &s.events {
            self.event(e);
        }
        for st in &s.states {
            for f in &st.functions {
                self.function(f);
            }
            for e in &st.events {
                self.event(e);
            }
        }
        for p in &s.properties {
            if let Some(g) = &p.getter {
                self.function(g);
            }
            if let Some(setter) = &p.setter {
                self.function(setter);
            }
        }
    }

    fn function(&mut self, f: &'a FunctionDef) {
        self.alloc(WalkNode::Function(f));
        self.body(&f.body);
    }

    fn event(&mut self, e: &'a EventDef) {
        self.alloc(WalkNode::Event(e));
        self.body(&e.body);
    }

    fn body(&mut self, stmts: &'a [Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match s {
            Stmt::ExprStmt { expr, .. } => self.expr(expr),
            Stmt::AssignStmt { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            Stmt::ReturnStmt { value: Some(v), .. } => self.expr(v),
            Stmt::ReturnStmt { value: None, .. } => {}
            Stmt::IfStmt {
                condition,
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                self.expr(condition);
                self.body(body);
                for c in elseif_clauses {
                    self.expr(&c.condition);
                    self.body(&c.body);
                }
                self.body(else_body);
            }
            Stmt::WhileStmt {
                condition, body, ..
            } => {
                self.expr(condition);
                self.body(body);
            }
            Stmt::LocalVarStmt {
                value: Some(init), ..
            } => self.expr(init),
            Stmt::LocalVarStmt { value: None, .. } => {}
        }
    }

    fn expr(&mut self, e: &'a Expr) {
        self.alloc(WalkNode::Expr(e));
        match e {
            Expr::CastExpr { expr, .. } => self.expr(expr),
            Expr::BinaryExpr { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::UnaryExpr { operand, .. } => self.expr(operand),
            Expr::CallExpr { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }
            Expr::DotCallExpr { object, args, .. } => {
                self.expr(object);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::DotExpr { object, .. } => self.expr(object),
            Expr::ArrayAccessExpr { array, index, .. } => {
                self.expr(array);
                self.expr(index);
            }
            Expr::NewArrayExpr { size, .. } => self.expr(size),
            Expr::NameExpr { .. } | Expr::LiteralExpr { .. } | Expr::ParentExpr { .. } => {}
        }
    }
}

// ---------------------------------------------------------------------------
// compile_source orchestrator
// ---------------------------------------------------------------------------

/// Outcome of compiling one script. `pex_bytes` is `None` on any error.
#[derive(Debug, Clone)]
pub struct CompileResult {
    pub ok: bool,
    pub pex_bytes: Option<Vec<u8>>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Parse → typeck → codegen → neutralize identity fields → serialize.
pub fn compile_source(
    text: &str,
    imports: &[String],
    game: Game,
    flags: Option<&str>,
) -> CompileResult {
    compile_source_with_path(text, imports, game, flags, None)
}

pub fn compile_source_with_path(
    text: &str,
    imports: &[String],
    game: Game,
    flags: Option<&str>,
    source_path: Option<&str>,
) -> CompileResult {
    let parsed = crate::parser::parse_script(text);
    if !parsed.errors.is_empty() {
        return CompileResult {
            ok: false,
            pex_bytes: None,
            diagnostics: parsed.errors.iter().map(diag_from_parse_error).collect(),
        };
    }
    let script_docstring = parsed.script_docstring.clone();
    let property_groups = parsed.property_groups.clone();
    let struct_names = parsed.struct_names.clone();
    let Some(ast) = parsed.ast else {
        return CompileResult {
            ok: false,
            pex_bytes: None,
            diagnostics: vec![error_diag(0, 0, "parser produced no AST")],
        };
    };

    // The compiler resolves cross-script/inherited references by parsing source
    // off the import path (the exe's `-i` dirs), never via the UI-only script_db.
    let resolver = crate::source_resolver::SourceResolver::with_self_ast(imports, &ast);
    let profile = GameProfile::for_game(game);
    let tc = crate::typeck::typeck(&ast, &resolver, profile);
    if tc
        .diagnostics
        .iter()
        .any(|d| matches!(d.severity, DiagnosticSeverity::Error))
    {
        return CompileResult {
            ok: false,
            pex_bytes: None,
            diagnostics: tc.diagnostics,
        };
    }

    let source_script_name = source_path_script_name(source_path, imports, &ast.name)
        .filter(|name| name.contains(':') && name.eq_ignore_ascii_case(&ast.name));
    let mut payload = crate::codegen::compile(
        &ast,
        &tc,
        &resolver,
        profile,
        flags,
        &script_docstring,
        &property_groups,
        &struct_names,
        source_script_name.as_deref(),
    );
    crate::pex_writer::neutralize_identity_fields(&mut payload);
    match crate::pex_writer::write_pex_bytes(&payload) {
        Ok(bytes) => CompileResult {
            ok: true,
            pex_bytes: Some(bytes),
            diagnostics: tc.diagnostics,
        },
        Err(e) => CompileResult {
            ok: false,
            pex_bytes: None,
            diagnostics: vec![error_diag(0, 0, &format!("writer error: {e}"))],
        },
    }
}

fn source_path_script_name(
    source_path: Option<&str>,
    imports: &[String],
    declared_name: &str,
) -> Option<String> {
    let path = Path::new(source_path?);
    let rel = imports
        .iter()
        .map(Path::new)
        .find_map(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let mut parts: Vec<String> = Vec::new();
    for component in rel.with_extension("").components() {
        if let Component::Normal(part) = component {
            parts.push(part.to_string_lossy().to_string());
        }
    }
    let path_stem = parts.last()?;
    let declared_stem = declared_name.rsplit(':').next().unwrap_or(declared_name);
    if parts.len() > 1
        && path_stem.eq_ignore_ascii_case(declared_stem)
        && path_stem != declared_stem
    {
        Some(parts.join(":").to_ascii_lowercase())
    } else {
        None
    }
}

fn error_diag(line: u32, col: u32, message: &str) -> Diagnostic {
    Diagnostic {
        line,
        col,
        end_line: line,
        end_col: col,
        message: message.to_string(),
        severity: DiagnosticSeverity::Error,
    }
}

fn diag_from_parse_error(e: &crate::parser::ParseError) -> Diagnostic {
    Diagnostic {
        line: e.line,
        col: e.col,
        end_line: e.line,
        end_col: e.col,
        message: e.message.clone(),
        severity: DiagnosticSeverity::Error,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fo4() -> GameProfile {
        GameProfile::for_game(Game::Fo4)
    }

    /// `typeck` (which assigns NodeIds via `walk_preorder`) and the shared walk
    /// that `codegen` consumes must agree node-for-node.
    #[test]
    fn typeck_and_codegen_share_node_ids() {
        let src = "ScriptName T\n\
                   Int Function F(Int a)\n  Int x = a + 1\n  Return x\nEndFunction\n\
                   Function G()\n  Int y = 2 * 3\nEndFunction\n";
        let parsed = crate::parser::parse_script(src);
        let ast = parsed.ast.expect("ast");

        let mut walk_log: Vec<(NodeId, String)> = Vec::new();
        walk_preorder(&ast, |id, node| walk_log.push((id, fingerprint(&node))));

        let resolver = crate::source_resolver::SourceResolver::new(&[]);
        let tc = crate::typeck::typeck(&ast, &resolver, fo4());

        assert_eq!(
            tc.node_order, walk_log,
            "typeck inference order must match the shared codegen walk"
        );
        // Sanity: a non-trivial fixture really exercised multiple functions.
        assert!(walk_log.len() >= 10, "fixture too small: {walk_log:?}");
    }

    #[test]
    fn compile_source_trivial_round_trips() {
        let r = compile_source("ScriptName TinyTest extends Quest\n", &[], Game::Fo4, None);
        assert!(r.ok, "{:?}", r.diagnostics);
        let bytes = r.pex_bytes.expect("bytes");
        assert_eq!(&bytes[..4], &crate::pex::PEX_MAGIC.to_le_bytes());
        let reparsed = crate::pex::parse_pex_bytes(&bytes).expect("reparse");
        assert_eq!(reparsed.objects[0].name, "TinyTest");
        assert_eq!(reparsed.objects[0].parent, "Quest");
    }

    #[test]
    fn compile_source_reports_parse_errors() {
        let r = compile_source("Not a script at all {", &[], Game::Fo4, None);
        assert!(!r.ok);
        assert!(!r.diagnostics.is_empty());
        assert!(r.pex_bytes.is_none());
    }

    #[test]
    fn compile_source_rejects_mismatched_inherited_event_signature() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let import_dir = std::env::temp_dir().join(format!(
            "papyrus_event_signature_{}_{}",
            std::process::id(),
            unique,
        ));
        std::fs::create_dir_all(&import_dir).expect("create import dir");
        std::fs::write(
            import_dir.join("TopicInfoSignatureParent.psc"),
            "ScriptName TopicInfoSignatureParent\n\
             Event OnBegin(ObjectReference akSpeakerRef, Bool abHasBeenSaid)\n\
             EndEvent\n",
        )
        .expect("write parent source");
        let imports = vec![import_dir.to_string_lossy().into_owned()];

        let mismatch = compile_source(
            "ScriptName TopicInfoSignatureMismatch Extends TopicInfoSignatureParent\n\
             Event OnBegin(ObjectReference akSpeakerRef, ObjectReference akTargetRef, Quest akQuestInstance, Bool abHasBeenSaid)\n\
             EndEvent\n",
            &imports,
            Game::Fo4,
            None,
        );
        let matching = compile_source(
            "ScriptName TopicInfoSignatureMatch Extends TopicInfoSignatureParent\n\
             Event OnBegin(ObjectReference akSpeakerRef, Bool abHasBeenSaid)\n\
             EndEvent\n",
            &imports,
            Game::Fo4,
            None,
        );
        std::fs::remove_dir_all(&import_dir).expect("remove import dir");

        assert!(!mismatch.ok, "{:?}", mismatch.diagnostics);
        assert!(
            mismatch
                .diagnostics
                .iter()
                .any(|d| d.message.contains("does not match inherited signature")),
            "{:?}",
            mismatch.diagnostics,
        );
        assert!(matching.ok, "{:?}", matching.diagnostics);
    }

    #[test]
    fn compile_source_types_inherited_properties_structs_and_namespaced_custom_events() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let import_dir = std::env::temp_dir().join(format!(
            "papyrus_inherited_property_{}_{}",
            std::process::id(),
            unique,
        ));
        std::fs::create_dir_all(import_dir.join("InheritNs")).expect("create import dir");
        std::fs::write(
            import_dir.join("InheritNs").join("Timer.psc"),
            "ScriptName InheritNs:Timer Extends Quest\n\
             CustomEvent Ended\n\
             Struct Pair\n\
               Int Regular = -1\n\
             EndStruct\n\
             Int Property StateIndex Auto\n\
             Pair[] Property Pairs Auto\n",
        )
        .expect("write parent source");
        let imports = vec![import_dir.to_string_lossy().into_owned()];

        let result = compile_source(
            "ScriptName InheritNs:TimerUser Extends InheritNs:Timer\n\
             Int Function CurrentState()\n\
               Int fromName = StateIndex\n\
               Return Self.StateIndex + fromName\n\
             EndFunction\n\
             Int Function FirstRegular(InheritNs:Timer akTimer)\n\
               Int regular = akTimer.Pairs[0].Regular\n\
               Return regular\n\
             EndFunction\n\
             Function Listen(InheritNs:Timer akTimer)\n\
               RegisterForCustomEvent(akTimer, \"Ended\")\n\
             EndFunction\n\
             Event InheritNs:Timer.Ended(InheritNs:Timer akSender, Var[] akArgs)\n\
             EndEvent\n",
            &imports,
            Game::Fo4,
            None,
        );
        std::fs::remove_dir_all(&import_dir).expect("remove import dir");

        assert!(result.ok, "{:?}", result.diagnostics);
    }

    /// The script under compilation is in memory, not on the import path; its
    /// own hierarchy must still resolve so same-script calls get their declared
    /// return type instead of `None`.
    #[test]
    fn same_script_call_resolves_return_type_for_every_type() {
        for (ret_ty, ret_expr) in [
            ("Int", "1"),
            ("Float", "1.0"),
            ("Bool", "True"),
            ("String", "\"a\""),
            ("ObjectReference", "None"),
        ] {
            for call in ["H()", "Self.H()"] {
                let src = format!(
                    "ScriptName SelfCallReturn Extends Quest\n\
                     {ret_ty} Function H()\n  Return {ret_expr}\nEndFunction\n\
                     Function C()\n  {ret_ty} x = {call}\nEndFunction\n"
                );
                let r = compile_source(&src, &[], Game::Fo4, None);
                assert!(r.ok, "{ret_ty} via {call}: {:?}", r.diagnostics);
            }
        }
    }

    #[test]
    fn same_script_call_resolves_array_return_type() {
        let r = compile_source(
            "ScriptName SelfCallArrayReturn Extends Quest\n\
             Int[] Function H()\n  Int[] a = new Int[1]\n  Return a\nEndFunction\n\
             Function C()\n  Int[] x = H()\n  Int y = x[0]\nEndFunction\n",
            &[],
            Game::Fo4,
            None,
        );
        assert!(r.ok, "{:?}", r.diagnostics);
    }

    /// The self-AST seeding must not make the resolver blind to real type
    /// errors: a mistyped assignment from a resolved self call still fails.
    #[test]
    fn same_script_call_return_type_is_checked_not_ignored() {
        let r = compile_source(
            "ScriptName SelfCallMistyped Extends Quest\n\
             ObjectReference Function H()\n  Return None\nEndFunction\n\
             Function C()\n  Int x = H()\nEndFunction\n",
            &[],
            Game::Fo4,
            None,
        );
        assert!(!r.ok, "expected a type error, got {:?}", r.diagnostics);
    }

    /// Inherited functions resolve through the seeded script's `Extends` chain.
    #[test]
    fn parent_script_call_resolves_return_type() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let import_dir = std::env::temp_dir().join(format!(
            "papyrus_parent_return_{}_{}",
            std::process::id(),
            unique,
        ));
        std::fs::create_dir_all(&import_dir).expect("create import dir");
        std::fs::write(
            import_dir.join("ParentReturnBase.psc"),
            "ScriptName ParentReturnBase\nInt Function GetCount()\n  Return 0\nEndFunction\n",
        )
        .expect("write parent source");
        let imports = vec![import_dir.to_string_lossy().into_owned()];

        let r = compile_source(
            "ScriptName ParentReturnChild Extends ParentReturnBase\n\
             Function C()\n  Int x = GetCount()\nEndFunction\n",
            &imports,
            Game::Fo4,
            None,
        );
        std::fs::remove_dir_all(&import_dir).expect("remove import dir");
        assert!(r.ok, "{:?}", r.diagnostics);
    }

    /// A minimal FO4 API with a complete chain to `ScriptObject`, so the
    /// checker can prove a member absent.
    fn semantic_api_dir(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "papyrus_semantic_{tag}_{}_{}",
            std::process::id(),
            unique,
        ));
        std::fs::create_dir_all(&dir).expect("create import dir");
        for (name, src) in [
            (
                "ScriptObject",
                "ScriptName ScriptObject Native Hidden\n\
                 Function GotoState(String asNewState) native\n\
                 Function RegisterForRemoteEvent(ScriptObject akSender, ScriptEventName asEventName) native\n\
                 Function RegisterForCustomEvent(ScriptObject akSender, CustomEventName asEventName) native\n\
                 Function SendCustomEvent(CustomEventName asEventName, Var[] akArgs = None) native\n\
                 Event OnInit()\nEndEvent\n",
            ),
            (
                "Form",
                "ScriptName Form Native Hidden\nInt Function GetFormID() native\n",
            ),
            ("Location", "ScriptName Location extends Form Native Hidden\n"),
            (
                "Keyword",
                "ScriptName Keyword extends Form Native Hidden\n\
                 Function SendStoryEvent(Location akLoc = None, ObjectReference akRef1 = None, ObjectReference akRef2 = None) native\n",
            ),
            (
                "ObjectReference",
                "ScriptName ObjectReference extends Form Native Hidden\n\
                 Function MoveTo(ObjectReference akTarget, Float afXOffset = 0.0, Bool abMatchRotation = True) native\n\
                 Event OnLoad()\nEndEvent\n",
            ),
            (
                "Quest",
                "ScriptName Quest extends Form Native Hidden\n\
                 Int Property Count Auto\n\
                 Bool Function SetStage(Int aiStage) native\n",
            ),
            (
                "Game",
                "ScriptName Game Native Hidden\nObjectReference Function GetPlayer() global native\n",
            ),
        ] {
            std::fs::write(dir.join(format!("{name}.psc")), src).expect("write api source");
        }
        dir
    }

    fn compile_quest_body(dir: &std::path::Path, body: &str) -> CompileResult {
        let src = format!(
            "ScriptName SemanticCheck extends Quest\n\
             CustomEvent Ready\n\
             Keyword Property Kw Auto\n\
             Function F()\n{body}\nEndFunction\n"
        );
        compile_source(
            &src,
            &[dir.to_string_lossy().into_owned()],
            Game::Fo4,
            None,
        )
    }

    #[test]
    fn rejects_what_the_stock_compiler_rejects() {
        let dir = semantic_api_dir("reject");
        let cases = [
            ("NotARealFunc(5)", "NotARealFunc is not a function or does not exist"),
            ("Self.NotReal()", "NotReal is not a function or does not exist"),
            ("Parent.NotReal()", "NotReal is not a function or does not exist"),
            (
                "ObjectReference r = None\nr.NotReal()",
                "NotReal is not a function or does not exist on script ObjectReference",
            ),
            ("SetStage(\"a\")", "type mismatch on parameter 1 (aiStage)"),
            ("SetStage(1, 2)", "too many arguments passed to function SetStage"),
            ("SetStage()", "argument aiStage is not specified"),
            ("Int x = Unknown", "variable Unknown is undefined"),
            (
                "ObjectReference r = None\nInt x = r.NoProp",
                "NoProp is not a property on script ObjectReference",
            ),
            ("Int x = \"s\"", "cannot assign String to Int"),
            (
                "Game.GetPlayer().MoveTo(akBogus = None)",
                "akBogus is not a parameter of function",
            ),
            (
                "Location l = None\nKw.SendStoryEvent(akRef1 = l)",
                "type mismatch on parameter 2 (akRef1)",
            ),
            ("Quest.SetStage(1)", "cannot call the member function SetStage"),
            (
                "RegisterForRemoteEvent(Game.GetPlayer(), \"OnNope\")",
                "OnNope is not an event on ObjectReference or one of its parents",
            ),
            (
                "RegisterForCustomEvent(Self, \"Nope\")",
                "Nope is not a custom event on SemanticCheck or one of its parents",
            ),
            (
                "SendCustomEvent(\"Nope\")",
                "Nope is not a custom event on SemanticCheck or one of its parents",
            ),
        ];
        let results: Vec<_> = cases
            .iter()
            .map(|(body, _)| compile_quest_body(&dir, body))
            .collect();
        std::fs::remove_dir_all(&dir).expect("remove import dir");
        for ((body, needle), r) in cases.iter().zip(results) {
            assert!(!r.ok, "{body:?} compiled");
            assert!(
                r.diagnostics.iter().any(|d| d.message.contains(needle)),
                "{body:?}: expected {needle:?}, got {:?}",
                r.diagnostics
            );
        }
    }

    #[test]
    fn accepts_resolvable_calls_properties_and_conversions() {
        let dir = semantic_api_dir("accept");
        let r = compile_quest_body(
            &dir,
            "ObjectReference p = Game.GetPlayer()\n\
             p.MoveTo(p, abMatchRotation = False)\n\
             Kw.SendStoryEvent(akRef1 = p)\n\
             SetStage(1)\n\
             Self.SetStage(1)\n\
             RegisterForRemoteEvent(p, \"OnLoad\")\n\
             RegisterForCustomEvent(Self, \"Ready\")\n\
             SendCustomEvent(\"Ready\")\n\
             GotoState(\"Busy\")\n\
             OnInit()\n\
             Int n = Count + Self.Count + GetFormID()",
        );
        std::fs::remove_dir_all(&dir).expect("remove import dir");
        assert!(r.ok, "{:?}", r.diagnostics);
    }

    /// Without the parent on the import path nothing can be proven missing.
    #[test]
    fn incomplete_parent_chain_reports_no_unresolved_members() {
        let r = compile_source(
            "ScriptName OrphanCheck extends MissingParent\n\
             Function F()\n  Whatever(1)\n  Int y = z\n  Self.Other()\nEndFunction\n",
            &[],
            Game::Fo4,
            None,
        );
        assert!(r.ok, "{:?}", r.diagnostics);
    }

    /// `Kw.SendStoryEvent(akRef1 = p)` must land `p` in the second slot; the
    /// dot-call parser used to drop argument names and bind it to `akLoc`.
    #[test]
    fn dot_call_named_argument_binds_by_name() {
        let dir = semantic_api_dir("named");
        let r = compile_quest_body(
            &dir,
            "ObjectReference p = Game.GetPlayer()\nKw.SendStoryEvent(akRef1 = p)",
        );
        std::fs::remove_dir_all(&dir).expect("remove import dir");
        assert!(r.ok, "{:?}", r.diagnostics);
        let pex = crate::pex::parse_pex_bytes(&r.pex_bytes.expect("bytes")).expect("parse");
        let call = pex.objects[0]
            .states
            .iter()
            .flat_map(|s| &s.functions)
            .flat_map(|f| &f.instructions)
            .find(|i| {
                i.args
                    .first()
                    .and_then(|a| a.data.as_str())
                    .is_some_and(|m| m.eq_ignore_ascii_case("SendStoryEvent"))
            })
            .expect("SendStoryEvent call");
        // [method, receiver, dest, argc, akLoc, akRef1, akRef2]
        let slots = &call.args[4..];
        assert_eq!(slots.len(), 3, "{slots:?}");
        assert_eq!(slots[0].value_type, 0, "akLoc must be None: {slots:?}");
        assert_eq!(slots[1].data.as_str(), Some("p"), "{slots:?}");
    }

    #[test]
    fn source_path_script_name_lowercases_lowercase_file_stem() {
        let imports = vec!["User".to_string()];
        assert_eq!(
            source_path_script_name(
                Some("User/DailyOps_All/higheldermode.psc"),
                &imports,
                "DailyOps_All:HighElderMode",
            ),
            Some("dailyops_all:higheldermode".to_string())
        );
        assert_eq!(
            source_path_script_name(
                Some("User/CAMPPets/PetActorScript.psc"),
                &imports,
                "CAMPPets:PetActorScript",
            ),
            None
        );
        assert_eq!(
            source_path_script_name(
                Some("User/Quests/Storm/Encounters/rescript.psc"),
                &imports,
                "Quests:Storm:Encounters:rescript",
            ),
            None
        );
    }
}
