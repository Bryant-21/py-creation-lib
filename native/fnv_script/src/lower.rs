use crate::ast::{BinOp, Block, Expr, FunctionCall, LValue, Script, Stmt, UnaryOp, VarType};
use crate::context::{FnvScriptContext, PropertyKind, ScriptTarget};
use crate::error::FnvScriptError;
use crate::function_map::EntryShape;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
enum StaticKind {
    Int,
    Float,
    String,
    Bool,
    Record(String),
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub extends: String,
    pub vars: Vec<Var>,
    pub properties: Vec<Property>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub struct Var {
    pub name: String,
    pub ty: String,
    pub initial: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub ty: String,
    pub kind: PropertyKind,
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
    let declared_vars: HashMap<String, StaticKind> = script
        .variables
        .iter()
        .map(|var| (var.name.to_ascii_lowercase(), static_kind_for_var(var.ty)))
        .collect();
    let vars = Vec::new();
    let mut properties = script
        .variables
        .iter()
        .map(|var| {
            Ok(Property {
                name: var.name.clone(),
                ty: lower_var_type(var.ty),
                kind: PropertyKind::MutableState,
                initial: var
                    .initial
                    .as_ref()
                    .map(|initial| lower_constant_initializer(initial, var.ty))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, FnvScriptError>>()?;
    properties.extend(
        ctx.target
            .symbols()
            .filter(|symbol| symbol.property_kind != PropertyKind::Intrinsic)
            .map(|symbol| Property {
                name: symbol.papyrus_name.clone(),
                ty: symbol.papyrus_type.clone(),
                kind: symbol.property_kind,
                initial: None,
            }),
    );
    properties.sort_by(|left, right| left.name.cmp(&right.name));

    let mut events = Vec::new();
    let mut game_mode_body = Vec::new();
    for block in &script.blocks {
        if block.event.eq_ignore_ascii_case("GameMode") {
            for stmt in &block.statements {
                lower_stmt(stmt, ctx, &declared_vars, &mut game_mode_body)?;
            }
        } else {
            merge_event(&mut events, lower_block(block, ctx, &declared_vars)?)?;
        }
    }
    if !game_mode_body.is_empty() {
        lower_game_mode(&mut events, game_mode_body, ctx)?;
    }

    Ok(Module {
        name: ctx.script_class_name.clone(),
        extends: ctx.papyrus_extends.clone(),
        vars,
        properties,
        events,
    })
}

fn lower_constant_initializer(expr: &Expr, var_type: VarType) -> Result<String, FnvScriptError> {
    let compatible = matches!(
        (var_type, expr),
        (VarType::Int | VarType::Long | VarType::Short, Expr::Int(_))
            | (VarType::Float, Expr::Int(_) | Expr::Float(_))
            | (VarType::StringVar, Expr::String(_))
    );
    if !compatible {
        return Err(FnvScriptError::Unsupported {
            kind: "variable initializer",
            name: format!("{expr:?}"),
            reason: format!("initializer is not a constant compatible with {var_type:?}"),
        });
    }
    Ok(match expr {
        Expr::Int(value) => value.to_string(),
        Expr::Float(value) => value.to_string(),
        Expr::String(value) => format!("\"{}\"", value.replace('"', "\\\"")),
        _ => unreachable!(),
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

fn static_kind_for_var(var_type: VarType) -> StaticKind {
    match var_type {
        VarType::Int | VarType::Long | VarType::Short => StaticKind::Int,
        VarType::Float => StaticKind::Float,
        VarType::Ref => StaticKind::Record("object_reference".into()),
        VarType::StringVar => StaticKind::String,
    }
}

fn lower_block(
    block: &Block,
    ctx: &FnvScriptContext,
    declared_vars: &HashMap<String, StaticKind>,
) -> Result<Event, FnvScriptError> {
    let mut body = Vec::new();
    for stmt in &block.statements {
        lower_stmt(stmt, ctx, declared_vars, &mut body)?;
    }
    let signature = map_event_signature(&block.event)?;
    validate_event_target(&signature.name, ctx.target.base_class)?;
    if block.args.len() > 1 {
        return Err(FnvScriptError::Unsupported {
            kind: "event filter",
            name: block.event.clone(),
            reason: format!("expected at most one filter, got {}", block.args.len()),
        });
    }
    if let Some(filter) = block.args.first() {
        let Some(filter_param) = signature.filter_param else {
            return Err(FnvScriptError::Unsupported {
                kind: "event filter",
                name: block.event.clone(),
                reason: "FO4 event has no compatible source-reference filter".into(),
            });
        };
        body = vec![IrStmt::If {
            cond: format!(
                "({filter_param} == {})",
                lower_expr(filter, ctx, declared_vars)?
            ),
            then_branch: body,
            elif: Vec::new(),
            else_branch: Vec::new(),
        }];
    }
    Ok(Event {
        name: signature.name,
        params: signature.params,
        body,
    })
}

struct EventSignature {
    name: String,
    params: Vec<(String, String)>,
    filter_param: Option<&'static str>,
}

fn map_event_signature(name: &str) -> Result<EventSignature, FnvScriptError> {
    let signature = match name.to_ascii_lowercase().as_str() {
        "onactivate" => EventSignature {
            name: "OnActivate".into(),
            params: vec![("akActionRef".into(), "ObjectReference".into())],
            filter_param: Some("akActionRef"),
        },
        "ontriggerenter" => EventSignature {
            name: "OnTriggerEnter".into(),
            params: vec![("akActionRef".into(), "ObjectReference".into())],
            filter_param: Some("akActionRef"),
        },
        "ondeath" => EventSignature {
            name: "OnDeath".into(),
            params: vec![("akKiller".into(), "Actor".into())],
            filter_param: Some("akKiller"),
        },
        "onload" => EventSignature {
            name: "OnLoad".into(),
            params: Vec::new(),
            filter_param: None,
        },
        "oninit" => EventSignature {
            name: "OnInit".into(),
            params: Vec::new(),
            filter_param: None,
        },
        "onquestinit" => EventSignature {
            name: "OnQuestInit".into(),
            params: Vec::new(),
            filter_param: None,
        },
        _ => {
            return Err(FnvScriptError::Translate {
                kind: "event",
                name: name.to_string(),
            });
        }
    };
    Ok(signature)
}

fn validate_event_target(name: &str, target: ScriptTarget) -> Result<(), FnvScriptError> {
    let compatible = match name {
        "OnActivate" | "OnTriggerEnter" | "OnLoad" => matches!(
            target,
            ScriptTarget::Actor | ScriptTarget::ObjectReference | ScriptTarget::ReferenceAlias
        ),
        "OnDeath" => matches!(target, ScriptTarget::Actor | ScriptTarget::ReferenceAlias),
        "OnQuestInit" => target == ScriptTarget::Quest,
        "OnInit" => true,
        _ => false,
    };
    if compatible {
        Ok(())
    } else {
        Err(FnvScriptError::Unsupported {
            kind: "event target",
            name: name.into(),
            reason: format!("event is not valid for {target:?}"),
        })
    }
}

fn merge_event(events: &mut Vec<Event>, event: Event) -> Result<(), FnvScriptError> {
    if let Some(existing) = events
        .iter_mut()
        .find(|existing| existing.name.eq_ignore_ascii_case(&event.name))
    {
        if existing.params != event.params {
            return Err(FnvScriptError::Unsupported {
                kind: "event",
                name: event.name,
                reason: "duplicate event blocks lower to incompatible signatures".into(),
            });
        }
        existing.body.extend(event.body);
    } else {
        events.push(event);
    }
    Ok(())
}

fn lower_game_mode(
    events: &mut Vec<Event>,
    mut body: Vec<IrStmt>,
    ctx: &FnvScriptContext,
) -> Result<(), FnvScriptError> {
    if !ctx.target.game_mode_timer_seconds.is_finite() || ctx.target.game_mode_timer_seconds <= 0.0
    {
        return Err(FnvScriptError::Unsupported {
            kind: "GameMode timer",
            name: ctx.target.game_mode_timer_seconds.to_string(),
            reason: "timer interval must be a finite positive number of seconds".into(),
        });
    }
    let restart = timer_start(ctx);
    restart_before_returns(&mut body, &restart);
    body.push(IrStmt::Expr(restart.clone()));

    merge_event(
        events,
        Event {
            name: "OnTimer".into(),
            params: vec![("aiTimerID".into(), "Int".into())],
            body: vec![IrStmt::If {
                cond: format!("(aiTimerID == {})", ctx.target.game_mode_timer_id),
                then_branch: body,
                elif: Vec::new(),
                else_branch: Vec::new(),
            }],
        },
    )?;

    let lifecycle_events: &[&str] = match ctx.target.base_class {
        ScriptTarget::Quest => &["OnQuestInit"],
        ScriptTarget::Actor | ScriptTarget::ObjectReference | ScriptTarget::ReferenceAlias => {
            &["OnInit", "OnLoad"]
        }
        ScriptTarget::MagicEffect | ScriptTarget::Other => {
            return Err(FnvScriptError::Unsupported {
                kind: "GameMode",
                name: ctx.papyrus_extends.clone(),
                reason: "target base class has no verified recurring-timer lifecycle".into(),
            });
        }
    };

    for event_name in lifecycle_events {
        if let Some(event) = events
            .iter_mut()
            .find(|event| event.name.eq_ignore_ascii_case(event_name))
        {
            event.body.insert(0, IrStmt::Expr(restart.clone()));
        } else {
            events.push(Event {
                name: (*event_name).into(),
                params: Vec::new(),
                body: vec![IrStmt::Expr(restart.clone())],
            });
        }
    }
    Ok(())
}

fn timer_start(ctx: &FnvScriptContext) -> String {
    format!(
        "StartTimer({}, {})",
        format_float(ctx.target.game_mode_timer_seconds),
        ctx.target.game_mode_timer_id
    )
}

fn format_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn restart_before_returns(statements: &mut Vec<IrStmt>, restart: &str) {
    let mut index = 0;
    while index < statements.len() {
        match &mut statements[index] {
            IrStmt::Return => {
                statements.insert(index, IrStmt::Expr(restart.to_string()));
                index += 2;
            }
            IrStmt::If {
                then_branch,
                elif,
                else_branch,
                ..
            } => {
                restart_before_returns(then_branch, restart);
                for (_, body) in elif {
                    restart_before_returns(body, restart);
                }
                restart_before_returns(else_branch, restart);
                index += 1;
            }
            _ => index += 1,
        }
    }
}

fn lower_stmt(
    stmt: &Stmt,
    ctx: &FnvScriptContext,
    declared_vars: &HashMap<String, StaticKind>,
    out: &mut Vec<IrStmt>,
) -> Result<(), FnvScriptError> {
    match stmt {
        Stmt::Set { target, value } => {
            let target = match target {
                LValue::Var(name) => name.clone(),
                LValue::Member { receiver, name } => {
                    lower_member(receiver, name, ctx, declared_vars)?
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
    declared_vars: &HashMap<String, StaticKind>,
) -> Result<String, FnvScriptError> {
    match expr {
        Expr::Int(value) => Ok(value.to_string()),
        Expr::Float(value) => Ok(value.to_string()),
        Expr::String(value) => Ok(format!("\"{}\"", value.replace('"', "\\\""))),
        Expr::Ident(name) => lower_ident(name, ctx, declared_vars),
        Expr::Member { receiver, name } => {
            if let Some(entry) = ctx.function_map.get(name)
                && entry.arg_kinds.is_empty()
            {
                return lower_call(
                    &FunctionCall {
                        name: name.clone(),
                        receiver: Some(receiver.clone()),
                        args: Vec::new(),
                    },
                    ctx,
                    declared_vars,
                );
            }
            lower_member(receiver, name, ctx, declared_vars)
        }
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
    declared_vars: &HashMap<String, StaticKind>,
) -> Result<String, FnvScriptError> {
    if name.eq_ignore_ascii_case("self") || declared_vars.contains_key(&name.to_ascii_lowercase()) {
        return Ok(name.to_string());
    }
    if let Some(symbol) = ctx.target.symbol(name) {
        return Ok(symbol.papyrus_name.clone());
    }
    match lower_zero_arg_symbol(name, ctx)? {
        Some(rewritten) => Ok(rewritten),
        None => Err(FnvScriptError::Translate {
            kind: "function",
            name: name.to_string(),
        }),
    }
}

fn lower_member(
    receiver: &Expr,
    name: &str,
    ctx: &FnvScriptContext,
    declared_vars: &HashMap<String, StaticKind>,
) -> Result<String, FnvScriptError> {
    let Expr::Ident(source_receiver) = receiver else {
        return Err(FnvScriptError::Unsupported {
            kind: "member",
            name: name.to_string(),
            reason: "nested member receiver has no target metadata".into(),
        });
    };
    let Some(symbol) = ctx.target.symbol(source_receiver) else {
        return Err(FnvScriptError::Translate {
            kind: "member receiver",
            name: source_receiver.clone(),
        });
    };
    let Some(member) = symbol.member(name) else {
        return Err(FnvScriptError::Translate {
            kind: "member",
            name: format!("{source_receiver}.{name}"),
        });
    };
    let lowered_receiver = lower_expr(receiver, ctx, declared_vars)?;
    Ok(format!("{lowered_receiver}.{member}"))
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
            if template.contains("{arg") {
                return Ok(None);
            }
            let rendered = template.replace("{self}", "Self");
            if rendered.lines().count() != 1 {
                return Err(FnvScriptError::Unsupported {
                    kind: "expression expansion",
                    name: entry.name.clone(),
                    reason: "multi-line expansion cannot be used as an expression".into(),
                });
            }
            Ok(Some(rendered))
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
    declared_vars: &HashMap<String, StaticKind>,
) -> Result<String, FnvScriptError> {
    let entry = ctx
        .function_map
        .get(&call.name)
        .ok_or_else(|| FnvScriptError::Translate {
            kind: "function",
            name: call.name.clone(),
        })?;

    if call.args.len() != entry.arg_kinds.len() {
        return Err(FnvScriptError::Unsupported {
            kind: "function arity",
            name: entry.name.clone(),
            reason: format!(
                "expected {} argument(s), got {}",
                entry.arg_kinds.len(),
                call.args.len()
            ),
        });
    }

    let self_value = match &call.receiver {
        Some(receiver) => lower_expr(receiver, ctx, declared_vars)?,
        None => "Self".into(),
    };
    let args = call
        .args
        .iter()
        .zip(&entry.arg_kinds)
        .map(|(expr, expected)| lower_typed_arg(expr, expected, ctx, declared_vars, &entry.name))
        .collect::<Result<Vec<_>, _>>()?;

    match &entry.shape {
        EntryShape::Papyrus { template } | EntryShape::Expansion { template } => {
            let mut rendered = template.clone();
            rendered = rendered.replace("{self}", &self_value);
            for (index, arg) in args.iter().enumerate() {
                rendered = rendered.replace(&format!("{{arg{index}}}"), arg);
            }
            if rendered.contains("{arg") {
                return Err(FnvScriptError::Unsupported {
                    kind: "function template",
                    name: entry.name.clone(),
                    reason: "unresolved argument placeholder after rendering".into(),
                });
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

fn lower_typed_arg(
    expr: &Expr,
    expected: &str,
    ctx: &FnvScriptContext,
    declared_vars: &HashMap<String, StaticKind>,
    function_name: &str,
) -> Result<String, FnvScriptError> {
    if expected == "actor_value" {
        return lower_actor_value(expr, ctx, function_name);
    }

    let actual = infer_static_kind(expr, ctx, declared_vars);
    match &actual {
        StaticKind::Unknown if !ctx.strict => {}
        StaticKind::Unknown => {
            return Err(argument_kind_error(
                function_name,
                expected,
                &format!(
                    "expression '{}' whose kind cannot be proven from the script context",
                    describe_expr(expr)
                ),
            ));
        }
        actual if !kind_is_compatible(expected, actual) => {
            return Err(argument_kind_error(
                function_name,
                expected,
                &format!("a statically known {} value", describe_static_kind(actual)),
            ));
        }
        _ => {}
    }
    lower_expr(expr, ctx, declared_vars)
}

fn lower_actor_value(
    expr: &Expr,
    ctx: &FnvScriptContext,
    function_name: &str,
) -> Result<String, FnvScriptError> {
    let Expr::Ident(source_name) = expr else {
        return Err(argument_kind_error(
            function_name,
            "actor_value",
            "a non-identifier expression",
        ));
    };
    let mapped = ctx
        .actor_value_map
        .get(&source_name.to_ascii_lowercase())
        .or_else(|| {
            ctx.actor_value_map
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(source_name))
                .map(|(_, mapped)| mapped)
        })
        .ok_or_else(|| FnvScriptError::Translate {
            kind: "actor value",
            name: source_name.clone(),
        })?;
    if !is_papyrus_identifier(mapped) {
        return Err(FnvScriptError::Unsupported {
            kind: "actor value mapping",
            name: source_name.clone(),
            reason: format!("mapped value '{mapped}' is not a Papyrus identifier"),
        });
    }
    Ok(mapped.clone())
}

fn infer_static_kind(
    expr: &Expr,
    ctx: &FnvScriptContext,
    declared_vars: &HashMap<String, StaticKind>,
) -> StaticKind {
    match expr {
        Expr::Int(_) => StaticKind::Int,
        Expr::Float(_) => StaticKind::Float,
        Expr::String(_) => StaticKind::String,
        Expr::Ident(name) => {
            if name.eq_ignore_ascii_case("self") {
                return static_kind_for_target(ctx.target.base_class);
            }
            if let Some(kind) = declared_vars.get(&name.to_ascii_lowercase()) {
                return kind.clone();
            }
            if let Some(symbol) = ctx.target.symbol(name) {
                if symbol.papyrus_name.eq_ignore_ascii_case("self") {
                    return static_kind_for_target(ctx.target.base_class);
                }
                if let Some(record_kind) = symbol.static_record_kind() {
                    return StaticKind::Record(record_kind.to_string());
                }
                return static_kind_for_papyrus_type(&symbol.papyrus_type);
            }
            ctx.function_map
                .get(name)
                .filter(|entry| entry.arg_kinds.is_empty())
                .map(|entry| static_kind_for_return_kind(&entry.return_kind))
                .unwrap_or(StaticKind::Unknown)
        }
        Expr::Member { .. } => StaticKind::Unknown,
        Expr::BinOp { op, lhs, rhs } => match op {
            BinOp::Eq
            | BinOp::Ne
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge
            | BinOp::And
            | BinOp::Or => StaticKind::Bool,
            BinOp::Add => match (
                infer_static_kind(lhs, ctx, declared_vars),
                infer_static_kind(rhs, ctx, declared_vars),
            ) {
                (StaticKind::String, _) | (_, StaticKind::String) => StaticKind::String,
                (StaticKind::Float, StaticKind::Int | StaticKind::Float)
                | (StaticKind::Int, StaticKind::Float) => StaticKind::Float,
                (StaticKind::Int, StaticKind::Int) => StaticKind::Int,
                _ => StaticKind::Unknown,
            },
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => match (
                infer_static_kind(lhs, ctx, declared_vars),
                infer_static_kind(rhs, ctx, declared_vars),
            ) {
                (StaticKind::Float, StaticKind::Int | StaticKind::Float)
                | (StaticKind::Int, StaticKind::Float) => StaticKind::Float,
                (StaticKind::Int, StaticKind::Int) => StaticKind::Int,
                _ => StaticKind::Unknown,
            },
        },
        Expr::UnaryOp { op, operand } => match op {
            UnaryOp::Not => StaticKind::Bool,
            UnaryOp::Neg => match infer_static_kind(operand, ctx, declared_vars) {
                StaticKind::Int => StaticKind::Int,
                StaticKind::Float => StaticKind::Float,
                _ => StaticKind::Unknown,
            },
        },
        Expr::Call(call) => ctx
            .function_map
            .get(&call.name)
            .map(|entry| static_kind_for_return_kind(&entry.return_kind))
            .unwrap_or(StaticKind::Unknown),
    }
}

fn static_kind_for_target(target: ScriptTarget) -> StaticKind {
    match target {
        ScriptTarget::Quest => StaticKind::Record("quest".into()),
        ScriptTarget::Actor => StaticKind::Record("actor".into()),
        ScriptTarget::ObjectReference | ScriptTarget::ReferenceAlias => {
            StaticKind::Record("object_reference".into())
        }
        ScriptTarget::MagicEffect => StaticKind::Record("magic_effect".into()),
        ScriptTarget::Other => StaticKind::Unknown,
    }
}

fn static_kind_for_papyrus_type(papyrus_type: &str) -> StaticKind {
    let normalized = papyrus_type.to_ascii_lowercase();
    let record_kind = match normalized.as_str() {
        "actor" => Some("actor"),
        "faction" => Some("faction"),
        "form" => Some("formkey"),
        "message" => Some("message"),
        "objectreference" | "referencealias" => Some("object_reference"),
        "package" => Some("package"),
        "perk" => Some("perk"),
        "quest" => Some("quest"),
        "topic" => Some("topic"),
        _ if normalized.ends_with("questscript") => Some("quest"),
        _ => None,
    };
    record_kind
        .map(|kind| StaticKind::Record(kind.into()))
        .unwrap_or(StaticKind::Unknown)
}

fn static_kind_for_return_kind(return_kind: &str) -> StaticKind {
    match return_kind.trim().to_ascii_lowercase().as_str() {
        "int" => StaticKind::Int,
        "float" => StaticKind::Float,
        "string" => StaticKind::String,
        "bool" => StaticKind::Bool,
        "actor" | "faction" | "formkey" | "message" | "object" | "object_reference" | "package"
        | "perk" | "quest" | "reputation" | "topic" => {
            StaticKind::Record(return_kind.trim().to_ascii_lowercase())
        }
        _ => StaticKind::Unknown,
    }
}

fn kind_is_compatible(expected: &str, actual: &StaticKind) -> bool {
    match (expected, actual) {
        ("int", StaticKind::Int)
        | ("float", StaticKind::Float | StaticKind::Int)
        | ("string", StaticKind::String)
        | ("bool", StaticKind::Bool | StaticKind::Int) => true,
        ("object", StaticKind::Record(actual)) => matches!(
            actual.as_str(),
            "actor" | "formkey" | "object" | "object_reference"
        ),
        ("object_reference", StaticKind::Record(actual)) => {
            matches!(actual.as_str(), "actor" | "object" | "object_reference")
        }
        ("formkey", StaticKind::Record(_)) => true,
        ("reputation", StaticKind::Record(actual)) => {
            matches!(actual.as_str(), "faction" | "reputation")
        }
        (expected, StaticKind::Record(actual)) => expected == actual,
        _ => false,
    }
}

fn describe_static_kind(kind: &StaticKind) -> &str {
    match kind {
        StaticKind::Int => "int",
        StaticKind::Float => "float",
        StaticKind::String => "string",
        StaticKind::Bool => "bool",
        StaticKind::Record(kind) => kind,
        StaticKind::Unknown => "unknown",
    }
}

fn argument_kind_error(function_name: &str, expected: &str, actual: &str) -> FnvScriptError {
    FnvScriptError::Unsupported {
        kind: "function argument",
        name: function_name.into(),
        reason: format!("expected {expected}, got {actual}"),
    }
}

fn is_papyrus_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn describe_expr(expr: &Expr) -> String {
    match expr {
        Expr::Int(value) => value.to_string(),
        Expr::Float(value) => value.to_string(),
        Expr::String(value) => format!("\"{value}\""),
        Expr::Ident(name) => name.clone(),
        Expr::Member { name, .. } => format!("<member>.{name}"),
        Expr::BinOp { .. } => "<binary expression>".into(),
        Expr::UnaryOp { .. } => "<unary expression>".into(),
        Expr::Call(call) => format!("{}(...)", call.name),
    }
}
