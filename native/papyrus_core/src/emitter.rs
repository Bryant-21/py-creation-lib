//! AST → Papyrus source text emitter.
//!
//! Mirror of `py_creation_lib/python/creation_lib/papyrus_lsp/emitter.py`. Used primarily by `py_creation_lib/python/creation_lib/pex/decompiler.py`
//! to render decompiled scripts. Pure formatter — no I/O, no GIL touches.

use crate::ast::*;

const PRECEDENCES: &[(&str, u8)] = &[
    ("||", 1),
    ("&&", 2),
    ("==", 3),
    ("!=", 3),
    ("<", 3),
    (">", 3),
    ("<=", 3),
    (">=", 3),
    ("+", 4),
    ("-", 4),
    ("*", 5),
    ("/", 5),
    ("%", 5),
];

fn precedence(op: &str) -> u8 {
    PRECEDENCES
        .iter()
        .find(|(o, _)| *o == op)
        .map(|(_, p)| *p)
        .unwrap_or(0)
}

pub fn emit_expr(node: &Expr) -> String {
    emit_expr_with_prec(node, 0)
}

fn emit_expr_with_prec(node: &Expr, parent_prec: u8) -> String {
    match node {
        Expr::NameExpr { name, .. } => name.clone(),
        Expr::LiteralExpr { value, ty, .. } => match (ty.as_str(), value) {
            ("string", LiteralValue::Str(s)) => format!("\"{s}\""),
            ("bool", LiteralValue::Bool(true)) => "True".into(),
            ("bool", LiteralValue::Bool(false)) => "False".into(),
            ("none", _) => "None".into(),
            ("float", LiteralValue::Float(f)) => format_float(*f),
            ("int", LiteralValue::Int(i)) => i.to_string(),
            (_, LiteralValue::Int(i)) => i.to_string(),
            (_, LiteralValue::Float(f)) => format_float(*f),
            (_, LiteralValue::Str(s)) => s.clone(),
            (_, LiteralValue::Bool(b)) => {
                if *b {
                    "True".into()
                } else {
                    "False".into()
                }
            }
            (_, LiteralValue::Null) => "None".into(),
        },
        Expr::BinaryExpr {
            left, op, right, ..
        } => {
            let prec = precedence(op);
            let l = emit_expr_with_prec(left, prec);
            let r = emit_expr_with_prec(right, prec + 1);
            let s = format!("{l} {op} {r}");
            if prec < parent_prec {
                format!("({s})")
            } else {
                s
            }
        }
        Expr::UnaryExpr { op, operand, .. } => {
            format!("{op}{}", emit_expr_with_prec(operand, 100))
        }
        Expr::CastExpr {
            expr, target_type, ..
        } => {
            format!("{} as {target_type}", emit_operand(expr))
        }
        Expr::DotExpr { object, member, .. } => {
            format!("{}.{member}", emit_operand(object))
        }
        Expr::CallExpr { function, args, .. } => {
            if function.starts_with("new ") && args.is_empty() {
                return function.clone();
            }
            let a = args.iter().map(emit_expr).collect::<Vec<_>>().join(", ");
            format!("{function}({a})")
        }
        Expr::DotCallExpr {
            object,
            method,
            args,
            ..
        } => {
            let a = args.iter().map(emit_expr).collect::<Vec<_>>().join(", ");
            format!("{}.{method}({a})", emit_operand(object))
        }
        Expr::ArrayAccessExpr { array, index, .. } => {
            format!("{}[{}]", emit_operand(array), emit_expr(index))
        }
        Expr::NewArrayExpr {
            element_type, size, ..
        } => {
            format!("new {element_type}[{}]", emit_expr(size))
        }
        Expr::ParentExpr { .. } => "parent".into(),
    }
}

/// Render an operand that Papyrus parses more tightly than the surrounding
/// operator: the left side of `.` or `[]`, and the value being cast by `as`.
///
/// `.` and `[]` bind tighter than `as`, so a bare cast receiver reads as
/// `x as (Actor.EquipItem(...))` and is rejected with "unexpected token Dot in
/// statement". A cast atom also accepts exactly one `as`, so a chained cast has
/// to spell the inner one out: `(Self as Perk) as MyScript`. Unary and binary
/// operands need the same treatment in both positions.
fn emit_operand(node: &Expr) -> String {
    match node {
        Expr::CastExpr { .. } | Expr::BinaryExpr { .. } | Expr::UnaryExpr { .. } => {
            format!("({})", emit_expr(node))
        }
        _ => emit_expr(node),
    }
}

fn format_float(f: f64) -> String {
    // Matches Python's `str(float)` behavior closely enough for our needs:
    // produce a decimal point even for integral values.
    if f.fract() == 0.0 && f.is_finite() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

pub fn emit_stmt(node: &Stmt, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    match node {
        Stmt::AssignStmt {
            target, op, value, ..
        } => {
            format!("{prefix}{} {op} {}", emit_expr(target), emit_expr(value))
        }
        Stmt::ReturnStmt { value, .. } => match value {
            Some(v) => format!("{prefix}Return {}", emit_expr(v)),
            None => format!("{prefix}Return"),
        },
        Stmt::ExprStmt { expr, .. } => format!("{prefix}{}", emit_expr(expr)),
        Stmt::LocalVarStmt {
            name, ty, value, ..
        } => match value {
            Some(v) => format!("{prefix}{ty} {name} = {}", emit_expr(v)),
            None => format!("{prefix}{ty} {name}"),
        },
        Stmt::IfStmt {
            condition,
            body,
            elseif_clauses,
            else_body,
            ..
        } => {
            let mut lines = vec![format!("{prefix}If {}", emit_expr(condition))];
            if !body.is_empty() {
                lines.push(emit_body(body, indent + 1));
            }
            for c in elseif_clauses {
                lines.push(format!("{prefix}ElseIf {}", emit_expr(&c.condition)));
                if !c.body.is_empty() {
                    lines.push(emit_body(&c.body, indent + 1));
                }
            }
            if !else_body.is_empty() {
                lines.push(format!("{prefix}Else"));
                lines.push(emit_body(else_body, indent + 1));
            }
            lines.push(format!("{prefix}EndIf"));
            lines.join("\n")
        }
        Stmt::WhileStmt {
            condition, body, ..
        } => {
            let mut lines = vec![format!("{prefix}While {}", emit_expr(condition))];
            if !body.is_empty() {
                lines.push(emit_body(body, indent + 1));
            }
            lines.push(format!("{prefix}EndWhile"));
            lines.join("\n")
        }
    }
}

fn emit_body(stmts: &[Stmt], indent: usize) -> String {
    stmts
        .iter()
        .map(|s| emit_stmt(s, indent))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn emit_function(node: &FunctionDef, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    let params = node
        .params
        .iter()
        .map(|p| match &p.default {
            Some(d) => format!("{} {} = {}", p.ty, p.name, emit_expr(d)),
            None => format!("{} {}", p.ty, p.name),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let ret = if !node.return_type.is_empty() && node.return_type != "None" {
        format!("{} ", node.return_type)
    } else {
        String::new()
    };
    let mut suffix = String::new();
    if node.is_global {
        suffix.push_str(" Global");
    }
    if node.is_native {
        suffix.push_str(" Native");
    }
    let mut lines = vec![format!(
        "{prefix}{ret}Function {}({params}){suffix}",
        node.name
    )];
    if !node.docstring.is_empty() {
        lines.push(format!("{prefix}\t{{ {} }}", node.docstring));
    }
    if !node.is_native {
        if !node.body.is_empty() {
            lines.push(emit_body(&node.body, indent + 1));
        }
        lines.push(format!("{prefix}EndFunction"));
    }
    lines.join("\n")
}

pub fn emit_event(node: &EventDef, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    let params = node
        .params
        .iter()
        .map(|p| match &p.default {
            Some(d) => format!("{} {} = {}", p.ty, p.name, emit_expr(d)),
            None => format!("{} {}", p.ty, p.name),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let suffix = if node.is_native { " Native" } else { "" };
    let mut lines = vec![format!("{prefix}Event {}({params}){suffix}", node.name)];
    if !node.docstring.is_empty() {
        lines.push(format!("{prefix}\t{{ {} }}", node.docstring));
    }
    if !node.is_native {
        if !node.body.is_empty() {
            lines.push(emit_body(&node.body, indent + 1));
        }
        lines.push(format!("{prefix}EndEvent"));
    }
    lines.join("\n")
}

pub fn emit_property(node: &PropertyDef, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    let flag_str = node.flags.join(" ");
    let default = node
        .default
        .as_ref()
        .map(|d| format!(" = {}", emit_expr(d)))
        .unwrap_or_default();
    if node
        .flags
        .iter()
        .any(|f| f == "Auto" || f == "AutoReadOnly")
    {
        return format!(
            "{prefix}{} Property {}{default} {flag_str}",
            node.ty, node.name
        )
        .trim_end()
        .to_string();
    }
    let mut lines = vec![format!(
        "{prefix}{} Property {}{default}",
        node.ty, node.name
    )];
    if !node.docstring.is_empty() {
        lines.push(format!("{prefix}\t{{ {} }}", node.docstring));
    }
    if let Some(g) = &node.getter {
        lines.push(emit_function(g, indent + 1));
    }
    if let Some(s) = &node.setter {
        lines.push(emit_function(s, indent + 1));
    }
    lines.push(format!("{prefix}EndProperty"));
    lines.join("\n")
}

fn emit_variable(node: &VariableDef, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    let val = node
        .value
        .as_ref()
        .map(|v| format!(" = {}", emit_expr(v)))
        .unwrap_or_default();
    let flags = if node.flags.is_empty() {
        String::new()
    } else {
        format!(" {}", node.flags.join(" "))
    };
    format!("{prefix}{} {}{val}{flags}", node.ty, node.name)
}

fn emit_struct_member(node: &StructMemberDef, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    let val = node
        .value
        .as_ref()
        .map(|v| format!(" = {}", emit_expr(v)))
        .unwrap_or_default();
    let flags = if node.flags.is_empty() {
        String::new()
    } else {
        format!(" {}", node.flags.join(" "))
    };
    format!("{prefix}{} {}{val}{flags}", node.ty, node.name)
}

fn emit_struct(node: &StructDef, indent: usize) -> String {
    let prefix = "\t".repeat(indent);
    let mut lines = vec![format!("{prefix}Struct {}", node.name)];
    for member in &node.members {
        lines.push(emit_struct_member(member, indent + 1));
    }
    lines.push(format!("{prefix}EndStruct"));
    lines.join("\n")
}

pub fn emit_script(node: &ScriptNode) -> String {
    let mut lines = Vec::new();
    let mut header = format!("Scriptname {}", node.name);
    if let Some(parent) = &node.parent {
        header.push_str(&format!(" Extends {parent}"));
    }
    if !node.flags.is_empty() {
        header.push(' ');
        header.push_str(&node.flags.join(" "));
    }
    lines.push(header);
    lines.push(String::new());

    for imp in &node.imports {
        lines.push(format!("Import {}", imp.script_name));
    }
    if !node.imports.is_empty() {
        lines.push(String::new());
    }

    for s in &node.structs {
        lines.push(emit_struct(s, 0));
        lines.push(String::new());
    }

    for v in &node.variables {
        lines.push(emit_variable(v, 0));
    }
    if !node.variables.is_empty() {
        lines.push(String::new());
    }

    for p in &node.properties {
        lines.push(emit_property(p, 0));
    }
    if !node.properties.is_empty() {
        lines.push(String::new());
    }

    for f in &node.functions {
        lines.push(emit_function(f, 0));
        lines.push(String::new());
    }

    for e in &node.events {
        lines.push(emit_event(e, 0));
        lines.push(String::new());
    }

    for state in &node.states {
        let kw = if state.is_auto { "Auto State" } else { "State" };
        lines.push(format!("{kw} {}", state.name));
        for f in &state.functions {
            lines.push(emit_function(f, 1));
            lines.push(String::new());
        }
        for e in &state.events {
            lines.push(emit_event(e, 1));
            lines.push(String::new());
        }
        lines.push("EndState".into());
        lines.push(String::new());
    }

    let joined = lines.join("\n");
    let trimmed = joined.trim_end();
    format!("{trimmed}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_script;

    #[test]
    fn roundtrip_simple_script() {
        let src = "ScriptName Foo extends Bar\nInt Property X Auto\nFunction DoIt()\nEndFunction\n";
        let r = parse_script(src);
        let ast = r.ast.unwrap();
        let out = emit_script(&ast);
        assert!(out.contains("Scriptname Foo Extends Bar"));
        assert!(out.contains("Int Property X"));
        assert!(out.contains("Function DoIt"));
        assert!(out.contains("EndFunction"));
    }

    #[test]
    fn binary_expr_precedence() {
        let e = Expr::BinaryExpr {
            left: Box::new(Expr::BinaryExpr {
                left: Box::new(Expr::NameExpr {
                    name: "a".into(),
                    pos: Pos::ZERO,
                }),
                op: "+".into(),
                right: Box::new(Expr::NameExpr {
                    name: "b".into(),
                    pos: Pos::ZERO,
                }),
                pos: Pos::ZERO,
            }),
            op: "*".into(),
            right: Box::new(Expr::NameExpr {
                name: "c".into(),
                pos: Pos::ZERO,
            }),
            pos: Pos::ZERO,
        };
        // (a + b) * c — `+` has lower precedence than `*` so left side needs parens.
        assert_eq!(emit_expr(&e), "(a + b) * c");
    }

    #[test]
    fn dot_call_render() {
        let e = Expr::DotCallExpr {
            object: Box::new(Expr::NameExpr {
                name: "Game".into(),
                pos: Pos::ZERO,
            }),
            method: "GetPlayer".into(),
            args: vec![],
            pos: Pos::ZERO,
        };
        assert_eq!(emit_expr(&e), "Game.GetPlayer()");
    }

    fn name(n: &str) -> Expr {
        Expr::NameExpr {
            name: n.into(),
            pos: Pos::ZERO,
        }
    }

    fn cast(inner: Expr, ty: &str) -> Expr {
        Expr::CastExpr {
            expr: Box::new(inner),
            target_type: ty.into(),
            pos: Pos::ZERO,
        }
    }

    #[test]
    fn cast_receiver_of_dot_call_is_parenthesised() {
        let e = Expr::DotCallExpr {
            object: Box::new(cast(name("akActionRef"), "Actor")),
            method: "EquipItem".into(),
            args: vec![name("PipboyCharGen")],
            pos: Pos::ZERO,
        };
        assert_eq!(
            emit_expr(&e),
            "(akActionRef as Actor).EquipItem(PipboyCharGen)"
        );
    }

    #[test]
    fn chained_cast_spells_out_the_inner_cast() {
        let e = Expr::DotCallExpr {
            object: Box::new(cast(
                cast(name("Self"), "Perk"),
                "surv_collectwaterperkscript",
            )),
            method: "CollectDirtyWater".into(),
            args: vec![name("akActor")],
            pos: Pos::ZERO,
        };
        assert_eq!(
            emit_expr(&e),
            "((Self as Perk) as surv_collectwaterperkscript).CollectDirtyWater(akActor)"
        );
    }

    #[test]
    fn cast_receiver_of_dot_and_index_is_parenthesised() {
        let member = Expr::DotExpr {
            object: Box::new(cast(name("akTargetRef"), "Actor")),
            member: "myProp".into(),
            pos: Pos::ZERO,
        };
        assert_eq!(emit_expr(&member), "(akTargetRef as Actor).myProp");

        let index = Expr::ArrayAccessExpr {
            array: Box::new(cast(name("items"), "Form[]")),
            index: Box::new(Expr::LiteralExpr {
                value: LiteralValue::Int(0),
                ty: "int".into(),
                pos: Pos::ZERO,
            }),
            pos: Pos::ZERO,
        };
        assert_eq!(emit_expr(&index), "(items as Form[])[0]");
    }

    #[test]
    fn plain_receiver_keeps_no_parens() {
        let e = Expr::DotCallExpr {
            object: Box::new(Expr::DotCallExpr {
                object: Box::new(name("Game")),
                method: "GetPlayer".into(),
                args: vec![],
                pos: Pos::ZERO,
            }),
            method: "EvaluatePackage".into(),
            args: vec![],
            pos: Pos::ZERO,
        };
        assert_eq!(emit_expr(&e), "Game.GetPlayer().EvaluatePackage()");
    }

    #[test]
    fn cast_argument_is_not_parenthesised() {
        let e = Expr::DotCallExpr {
            object: Box::new(name("akActor")),
            method: "Revive".into(),
            args: vec![cast(name("akTargetRef"), "Actor")],
            pos: Pos::ZERO,
        };
        assert_eq!(emit_expr(&e), "akActor.Revive(akTargetRef as Actor)");
    }

    #[test]
    fn float_literal_with_decimal() {
        let e = Expr::LiteralExpr {
            value: LiteralValue::Float(2.0),
            ty: "float".into(),
            pos: Pos::ZERO,
        };
        assert_eq!(emit_expr(&e), "2.0");
    }
}
