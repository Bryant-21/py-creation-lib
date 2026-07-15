use crate::lower::{Event, IrStmt, Module};

pub fn emit_psc(module: &Module) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "ScriptName {} extends {}\n\n",
        module.name, module.extends
    ));
    for var in &module.vars {
        out.push_str(&format!("{} {}", var.ty, var.name));
        if let Some(initial) = &var.initial {
            out.push_str(&format!(" = {initial}"));
        }
        out.push('\n');
    }
    if !module.vars.is_empty() {
        out.push('\n');
    }
    for event in &module.events {
        emit_event(&mut out, event);
        out.push('\n');
    }
    out
}

fn emit_event(out: &mut String, event: &Event) {
    let params = event
        .params
        .iter()
        .map(|(name, ty)| format!("{ty} {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("Event {}({params})\n", event.name));
    for stmt in &event.body {
        emit_stmt(out, stmt, 1);
    }
    out.push_str("EndEvent\n");
}

fn emit_stmt(out: &mut String, stmt: &IrStmt, indent: usize) {
    let pad = "    ".repeat(indent);
    match stmt {
        IrStmt::Assign { target, value } => out.push_str(&format!("{pad}{target} = {value}\n")),
        IrStmt::Expr(expr) => out.push_str(&format!("{pad}{expr}\n")),
        IrStmt::If {
            cond,
            then_branch,
            elif,
            else_branch,
        } => {
            out.push_str(&format!("{pad}If {cond}\n"));
            for stmt in then_branch {
                emit_stmt(out, stmt, indent + 1);
            }
            for (elif_cond, body) in elif {
                out.push_str(&format!("{pad}ElseIf {elif_cond}\n"));
                for stmt in body {
                    emit_stmt(out, stmt, indent + 1);
                }
            }
            if !else_branch.is_empty() {
                out.push_str(&format!("{pad}Else\n"));
                for stmt in else_branch {
                    emit_stmt(out, stmt, indent + 1);
                }
            }
            out.push_str(&format!("{pad}EndIf\n"));
        }
        IrStmt::Return => out.push_str(&format!("{pad}Return\n")),
        IrStmt::Comment(comment) => out.push_str(&format!("{pad}; {comment}\n")),
    }
}
