use crate::ast::{BinOp, Block, Expr, FunctionCall, LValue, Script, Stmt, UnaryOp, VarType};
use crate::context::FnvScriptContext;
use crate::error::FnvScriptError;
use crate::function_map::EntryShape;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub extends: String,
    pub vars: Vec<Var>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub ty: String,
    pub initial: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub body: Vec<IrStmt>,
}

#[derive(Debug, Clone)]
pub enum IrStmt {
    Assign {
        target: String,
        value: String,
    },
    Expr(String),
    If {
        cond: String,
        then_branch: Vec<IrStmt>,
        elif: Vec<(String, Vec<IrStmt>)>,
        else_branch: Vec<IrStmt>,
    },
    Return,
    Comment(String),
}

pub fn lower(script: &Script, ctx: &FnvScriptContext) -> Result<Module, FnvScriptError> {
    let declared_vars: HashSet<String> = script
        .variables
        .iter()
        .map(|var| var.name.to_ascii_lowercase())
        .collect();
    let vars = script
        .variables
        .iter()
        .map(|var| Var {
            name: var.name.clone(),
            ty: lower_var_type(var.ty),
            initial: None,
        })
        .collect();

    let mut events = Vec::new();
    for block in &script.blocks {
        events.push(lower_block(block, ctx, &declared_vars)?);
    }

    Ok(Module {
        name: ctx.script_class_name.clone(),
        extends: ctx.papyrus_extends.clone(),
        vars,
        events,
    })
}

fn lower_var_type(var_type: VarType) -> String {
    match var_type {
        VarType::Int | VarType::Long | VarType::Short => "Int".into(),
        VarType::Float => "Float".into(),
        VarType::Ref => "ObjectReference".into(),
        VarType::StringVar => "String".into(),
    }
}

fn lower_block(
    block: &Block,
    ctx: &FnvScriptContext,
    declared_vars: &HashSet<String>,
) -> Result<Event, FnvScriptError> {
    let mut body = Vec::new();
    for stmt in &block.statements {
        lower_stmt(stmt, ctx, declared_vars, &mut body)?;
    }
    if block.event.eq_ignore_ascii_case("MenuMode") {
        body.insert(
            0,
            IrStmt::Comment("FNV MenuMode has no direct FO4 event equivalent".into()),
        );
    }
    let (name, params) = map_event_signature(&block.event);
    Ok(Event { name, params, body })
}

fn map_event_signature(name: &str) -> (String, Vec<(String, String)>) {
    match name.to_ascii_lowercase().as_str() {
        "onactivate" => (
            "OnActivate".into(),
            vec![("akActionRef".into(), "ObjectReference".into())],
        ),
        "ondeath" => ("OnDying".into(), Vec::new()),
        "onhit" => ("OnHit".into(), Vec::new()),
        "onequip" => ("OnEquipped".into(), Vec::new()),
        "onunequip" => ("OnUnequipped".into(), Vec::new()),
        "onload" => ("OnInit".into(), Vec::new()),
        "gamemode" => ("OnInit".into(), Vec::new()),
        "menumode" => ("OnMenuModeStub".into(), Vec::new()),
        other => (format!("On{}", capitalize(other)), Vec::new()),
    }
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn lower_stmt(
    stmt: &Stmt,
    ctx: &FnvScriptContext,
    declared_vars: &HashSet<String>,
    out: &mut Vec<IrStmt>,
) -> Result<(), FnvScriptError> {
    match stmt {
        Stmt::Set { target, value } => {
            let target = match target {
                LValue::Var(name) => name.clone(),
                LValue::Member { receiver, name } => {
                    format!("{}.{}", lower_expr(receiver, ctx, declared_vars)?, name)
                }
            };
            out.push(IrStmt::Assign {
                target,
                value: lower_expr(value, ctx, declared_vars)?,
            });
        }
        Stmt::If {
            cond,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            let mut then_ir = Vec::new();
            for stmt in then_branch {
                lower_stmt(stmt, ctx, declared_vars, &mut then_ir)?;
            }
            let mut elif = Vec::new();
            for (cond_expr, body) in elif_branches {
                let mut lowered = Vec::new();
                for stmt in body {
                    lower_stmt(stmt, ctx, declared_vars, &mut lowered)?;
                }
                elif.push((lower_expr(cond_expr, ctx, declared_vars)?, lowered));
            }
            let mut else_ir = Vec::new();
            for stmt in else_branch {
                lower_stmt(stmt, ctx, declared_vars, &mut else_ir)?;
            }
            out.push(IrStmt::If {
                cond: lower_expr(cond, ctx, declared_vars)?,
                then_branch: then_ir,
                elif,
                else_branch: else_ir,
            });
        }
        Stmt::Call(call) => {
            let lowered = lower_call(call, ctx, declared_vars)?;
            for line in lowered.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(comment) = trimmed.strip_prefix(';') {
                    out.push(IrStmt::Comment(comment.trim_start().to_string()));
                } else {
                    out.push(IrStmt::Expr(trimmed.to_string()));
                }
            }
        }
        Stmt::Return => out.push(IrStmt::Return),
        Stmt::ScriptBlockEnd => {}
    }
    Ok(())
}

fn lower_expr(
    expr: &Expr,
    ctx: &FnvScriptContext,
    declared_vars: &HashSet<String>,
) -> Result<String, FnvScriptError> {
    match expr {
        Expr::Int(value) => Ok(value.to_string()),
        Expr::Float(value) => Ok(value.to_string()),
        Expr::String(value) => Ok(format!("\"{}\"", value.replace('"', "\\\""))),
        Expr::Ident(name) => lower_ident(name, ctx, declared_vars),
        Expr::Member { receiver, name } => Ok(format!(
            "{}.{}",
            lower_expr(receiver, ctx, declared_vars)?,
            name
        )),
        Expr::BinOp { op, lhs, rhs } => Ok(format!(
            "({} {} {})",
            lower_expr(lhs, ctx, declared_vars)?,
            lower_binop(*op),
            lower_expr(rhs, ctx, declared_vars)?
        )),
        Expr::UnaryOp { op, operand } => Ok(format!(
            "{}{}",
            match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "!",
            },
            lower_expr(operand, ctx, declared_vars)?
        )),
        Expr::Call(call) => lower_call(call, ctx, declared_vars),
    }
}

fn lower_binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    }
}

fn lower_ident(
    name: &str,
    ctx: &FnvScriptContext,
    declared_vars: &HashSet<String>,
) -> Result<String, FnvScriptError> {
    if name.eq_ignore_ascii_case("self") || declared_vars.contains(&name.to_ascii_lowercase()) {
        return Ok(name.to_string());
    }
    match lower_zero_arg_symbol(name, ctx)? {
        Some(rewritten) => Ok(rewritten),
        None => Err(FnvScriptError::Translate {
            kind: "function",
            name: name.to_string(),
        }),
    }
}

fn lower_zero_arg_symbol(
    name: &str,
    ctx: &FnvScriptContext,
) -> Result<Option<String>, FnvScriptError> {
    let Some(entry) = ctx.function_map.get(name) else {
        return Ok(None);
    };
    if !entry.arg_kinds.is_empty() {
        return Ok(None);
    }

    match &entry.shape {
        EntryShape::Papyrus { template } | EntryShape::Expansion { template } => {
            if template.contains("{arg") || template.contains("{self}") {
                return Ok(None);
            }
            Ok(Some(template.clone()))
        }
        EntryShape::Drop { reason, .. } => Err(FnvScriptError::Drop {
            kind: "function",
            name: entry.name.clone(),
            reason: reason.clone(),
        }),
    }
}

fn lower_call(
    call: &FunctionCall,
    ctx: &FnvScriptContext,
    declared_vars: &HashSet<String>,
) -> Result<String, FnvScriptError> {
    let entry = ctx
        .function_map
        .get(&call.name)
        .ok_or_else(|| FnvScriptError::Translate {
            kind: "function",
            name: call.name.clone(),
        })?;

    let self_value = match &call.receiver {
        Some(receiver) => lower_expr(receiver, ctx, declared_vars)?,
        None => "Self".into(),
    };
    let args = call
        .args
        .iter()
        .map(|expr| lower_expr(expr, ctx, declared_vars))
        .collect::<Result<Vec<_>, _>>()?;

    match &entry.shape {
        EntryShape::Papyrus { template } | EntryShape::Expansion { template } => {
            let mut rendered = template.clone();
            rendered = rendered.replace("{self}", &self_value);
            for (index, arg) in args.iter().enumerate() {
                rendered = rendered.replace(&format!("{{arg{index}}}"), arg);
            }
            Ok(rendered)
        }
        EntryShape::Drop { reason, .. } => Err(FnvScriptError::Drop {
            kind: "function",
            name: entry.name.clone(),
            reason: reason.clone(),
        }),
    }
}
