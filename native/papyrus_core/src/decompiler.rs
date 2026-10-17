use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::ast::*;
use crate::pex::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DecompileOptions {
    pub type_aliases: HashMap<String, String>,
    pub drop_script_const: bool,
    pub skip_internal_functions: bool,
    pub fo4_api_compat: bool,
}

const P: Pos = Pos::ZERO;

fn name(value: impl Into<String>) -> Expr {
    Expr::NameExpr {
        name: value.into(),
        pos: P,
    }
}

fn literal(value: LiteralValue, ty: &str) -> Expr {
    Expr::LiteralExpr {
        value,
        ty: ty.into(),
        pos: P,
    }
}

fn assign(target: Expr, value: Expr) -> Stmt {
    Stmt::AssignStmt {
        target,
        op: "=".into(),
        value,
        pos: P,
    }
}

fn dot(object: Expr, member: impl Into<String>) -> Expr {
    Expr::DotExpr {
        object: Box::new(object),
        member: member.into(),
        pos: P,
    }
}

fn call(object: Expr, method: impl Into<String>, args: Vec<Expr>) -> Expr {
    Expr::DotCallExpr {
        object: Box::new(object),
        method: method.into(),
        args,
        arg_names: Vec::new(),
        pos: P,
    }
}

fn data_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "None".into(),
        serde_json::Value::Bool(true) => "True".into(),
        serde_json::Value::Bool(false) => "False".into(),
        value => value.to_string(),
    }
}

fn value_expr(value: &PexValuePayload) -> Expr {
    match value.value_type {
        0 => literal(LiteralValue::Null, "none"),
        1 => match value.data.as_str() {
            Some("self") => name("Self"),
            Some("::NoneVar") => literal(LiteralValue::Null, "none"),
            _ => name(data_string(&value.data)),
        },
        2 => literal(LiteralValue::Str(data_string(&value.data)), "string"),
        3 => literal(
            LiteralValue::Int(value.data.as_i64().unwrap_or_default()),
            "int",
        ),
        4 => literal(
            LiteralValue::Float(value.data.as_f64().unwrap_or_default()),
            "float",
        ),
        5 => literal(
            LiteralValue::Bool(value.data.as_bool().unwrap_or(false)),
            "bool",
        ),
        _ => name(data_string(&value.data)),
    }
}

fn default_expr(value: &PexValuePayload) -> Option<Expr> {
    (value.value_type != 0).then(|| value_expr(value))
}

fn is_temp(name: &str) -> bool {
    name.starts_with("::temp") || name.starts_with("::NoneVar") || name.starts_with("__temp")
}

fn source_name(name: &str) -> &str {
    name.strip_prefix("::").unwrap_or(name)
}

fn is_none_var(value: &PexValuePayload) -> bool {
    value.value_type == 1 && value.data.as_str() == Some("::NoneVar")
}

fn call_result(destination: &PexValuePayload, expr: Expr) -> Stmt {
    if is_none_var(destination) {
        Stmt::ExprStmt { expr, pos: P }
    } else {
        assign(value_expr(destination), expr)
    }
}

fn instruction(
    instruction: &PexInstructionPayload,
    types: &HashMap<String, String>,
    compat: bool,
) -> Result<Option<Stmt>, String> {
    let op = instruction.opcode;
    let a = &instruction.args;
    let minimum = match op {
        0 | 0x14..=0x16 | 0x1a => 0,
        0x0a..=0x0e | 0x1e..=0x1f => 2,
        0x25 => 1,
        0x17 | 0x19 => 4,
        0x18 => 3,
        0x01..=0x09 | 0x0f..=0x13 | 0x1b..=0x1d | 0x20..=0x21 | 0x24 | 0x26..=0x27 => 3,
        _ => 0,
    };
    if a.len() < minimum {
        return Err(format!(
            "PEX opcode {op:#x} needs {minimum} arguments, got {}",
            a.len()
        ));
    }
    let v = |i: usize| value_expr(&a[i]);
    let s = |i: usize| data_string(&a[i].data);
    let binary = match op {
        1 | 2 | 0x1b => Some("+"),
        3 | 4 => Some("-"),
        5 | 6 => Some("*"),
        7 | 8 => Some("/"),
        9 => Some("%"),
        0x0f => Some("=="),
        0x10 => Some("<"),
        0x11 => Some("<="),
        0x12 => Some(">"),
        0x13 => Some(">="),
        0x24 => Some("is"),
        _ => None,
    };
    if let Some(op) = binary {
        return Ok(Some(assign(
            v(0),
            Expr::BinaryExpr {
                left: Box::new(v(1)),
                op: op.into(),
                right: Box::new(v(2)),
                pos: P,
            },
        )));
    }
    Ok(match op {
        0 | 0x14..=0x16 => None,
        0x0a..=0x0c => Some(assign(
            v(0),
            Expr::UnaryExpr {
                op: if op == 0x0a { "!" } else { "-" }.into(),
                operand: Box::new(v(1)),
                pos: P,
            },
        )),
        0x0d => Some(assign(v(0), v(1))),
        0x0e => Some(assign(
            v(0),
            Expr::CastExpr {
                expr: Box::new(v(1)),
                target_type: types.get(&s(0)).cloned().unwrap_or("var".into()),
                pos: P,
            },
        )),
        0x17 => {
            let method = s(0);
            let lower = method.to_lowercase();
            let object_type = if a[1].value_type == 1 {
                types
                    .get(&s(1))
                    .map(|s| s.to_lowercase())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let mut raw: Vec<_> = a[4..].iter().collect();
            if compat {
                if (object_type.is_empty() || object_type == "sound")
                    && matches!(lower.as_str(), "play" | "playandwait")
                    && raw.last().is_some_and(|v| v.value_type == 2)
                {
                    raw.pop();
                }
                if matches!(object_type.as_str(), "" | "objectreference" | "actor")
                    && lower == "placeatme"
                    && raw.len() == 7
                {
                    raw.truncate(5);
                }
                if matches!(object_type.as_str(), "" | "actor")
                    && lower == "dismember"
                    && raw.len() == 5
                {
                    raw.truncate(4);
                }
                if object_type == "weapon"
                    && lower == "fire"
                    && raw.len() == 3
                    && raw[2].value_type == 5
                {
                    raw.pop();
                }
                if matches!(object_type.as_str(), "" | "objectreference" | "actor")
                    && lower == "getrefslinkedtome"
                    && raw.len() == 3
                    && raw[2].value_type == 5
                {
                    raw.pop();
                }
                if matches!(
                    lower.as_str(),
                    "registerforhitevent" | "unregisterforhitevent"
                ) && raw.len() == 10
                    && raw[8].value_type == 3
                    && raw[9].value_type == 5
                {
                    raw.remove(8);
                }
                if (matches!(object_type.as_str(), "" | "actor")
                    && lower == "setequippedweaponattacksenabled")
                    || matches!(
                        lower.as_str(),
                        "registerfordamagedealtevent"
                            | "unregisterfordamagedealtevent"
                            | "unregisterforalldamagedealtevents"
                    )
                {
                    return Ok(None);
                }
                // The FO76 compiler always emits full arity, so these counts are
                // the FO76 signatures; the extra trailing parameters (stash,
                // instancing, persistence flags) have no FO4 counterpart.
                let fo4_arity = match (lower.as_str(), raw.len()) {
                    ("additem", 6) => Some(3),
                    ("removeitem", 5) => Some(4),
                    ("getitemcount", 2) => Some(1),
                    ("findallreferenceswithkeyword", 3) => Some(2),
                    ("placeactoratme", 6) => Some(2),
                    ("mute" | "unmute", 1)
                        if matches!(object_type.as_str(), "" | "soundcategory") =>
                    {
                        Some(0)
                    }
                    _ => None,
                };
                if let Some(arity) = fo4_arity {
                    raw.truncate(arity);
                }
                // FO76 targets help messages at a player[]; FO4 has one player.
                if lower == "showashelpmessage"
                    && raw.len() == 7
                    && matches!(object_type.as_str(), "" | "message")
                {
                    raw.remove(0);
                }
                match (lower.as_str(), raw.len()) {
                    ("setbloodimpactmaterial", 1) | ("requestreconmarkonserver", 0) => {
                        return Ok(None);
                    }
                    // Single-player FO4 has exactly one player.
                    ("islocalplayer" | "isaplayer", 0) => {
                        return Ok(Some(call_result(
                            &a[2],
                            Expr::BinaryExpr {
                                left: Box::new(v(1)),
                                op: "==".into(),
                                right: Box::new(call(name("Game"), "GetPlayer", vec![])),
                                pos: P,
                            },
                        )));
                    }
                    // The "server" is this same script.
                    ("sendrmitoserver", 2) => {
                        return Ok(Some(call_result(
                            &a[2],
                            call(v(1), "CallFunction", raw.into_iter().map(value_expr).collect()),
                        )));
                    }
                    ("findclosestreferenceoftype", 2) => {
                        return Ok(Some(call_result(
                            &a[2],
                            call(
                                name("Game"),
                                "FindClosestReferenceOfTypeFromRef",
                                vec![value_expr(raw[0]), v(1), value_expr(raw[1])],
                            ),
                        )));
                    }
                    // FO76 revives a downed target; FO4's nearest is resurrecting it.
                    ("revive", 2) => {
                        let target = Expr::CastExpr {
                            expr: Box::new(value_expr(raw[0])),
                            target_type: "Actor".into(),
                            pos: P,
                        };
                        return Ok(Some(call_result(&a[2], call(target, "Resurrect", vec![]))));
                    }
                    _ => {}
                }
            }
            Some(call_result(
                &a[2],
                call(v(1), method, raw.into_iter().map(value_expr).collect()),
            ))
        }
        0x18 => Some(call_result(
            &a[1],
            call(
                Expr::ParentExpr { pos: P },
                s(0),
                a[3..].iter().map(value_expr).collect(),
            ),
        )),
        0x19 => {
            let class = s(0);
            let mut method = s(1);
            let mut raw = &a[4..];
            if compat
                && class.eq_ignore_ascii_case("game")
                && method.eq_ignore_ascii_case("getlocalplayer")
            {
                method = "GetPlayer".into();
            }
            if compat {
                raw = fo4_static_args(&class, &method, raw);
            }
            // TraceLog(CallingObject, Text, LogName, DejaChannel, Severity, ShowNormal)
            if compat
                && class.eq_ignore_ascii_case("debug")
                && method.eq_ignore_ascii_case("tracelog")
                && raw.len() == 6
            {
                return Ok(Some(call_result(
                    &a[2],
                    call(
                        name("Debug"),
                        "TraceUser",
                        vec![value_expr(&raw[2]), value_expr(&raw[1]), value_expr(&raw[4])],
                    ),
                )));
            }
            if compat
                && class.eq_ignore_ascii_case("player")
                && (method.eq_ignore_ascii_case("shakecamera")
                    || method.eq_ignore_ascii_case("shakecontroller"))
            {
                return Ok(Some(call_result(
                    &a[2],
                    call(name("Game"), method, raw.iter().map(value_expr).collect()),
                )));
            }
            Some(call_result(
                &a[2],
                call(name(class), method, raw.iter().map(value_expr).collect()),
            ))
        }
        0x1a => Some(Stmt::ReturnStmt {
            value: a.first().filter(|v| !is_none_var(v)).map(value_expr),
            pos: P,
        }),
        0x1c => Some(assign(v(2), dot(v(1), s(0)))),
        0x1d => Some(assign(dot(v(1), s(0)), v(2))),
        0x1e => {
            let ty = types.get(&s(0)).map(String::as_str).unwrap_or("var");
            Some(assign(
                v(0),
                Expr::NewArrayExpr {
                    element_type: ty.strip_suffix("[]").unwrap_or(ty).into(),
                    size: Box::new(v(1)),
                    pos: P,
                },
            ))
        }
        0x1f => Some(assign(v(0), dot(v(1), "Length"))),
        0x20 => Some(assign(
            v(0),
            Expr::ArrayAccessExpr {
                array: Box::new(v(1)),
                index: Box::new(v(2)),
                pos: P,
            },
        )),
        0x21 => Some(assign(
            Expr::ArrayAccessExpr {
                array: Box::new(v(0)),
                index: Box::new(v(1)),
                pos: P,
            },
            v(2),
        )),
        0x25 => Some(assign(
            v(0),
            Expr::CallExpr {
                function: format!(
                    "new {}",
                    types.get(&s(0)).map(String::as_str).unwrap_or("var")
                ),
                args: vec![],
                arg_names: vec![],
                pos: P,
            },
        )),
        0x26 => Some(assign(v(0), dot(v(1), s(2)))),
        0x27 => Some(assign(dot(v(0), s(1)), v(2))),
        _ => {
            let opcode_name = match op {
                0x22 => "ARRAY_FINDELEMENT",
                0x23 => "ARRAY_RFINDELEMENT",
                0x28 => "ARRAY_FINDSTRUCT",
                0x29 => "ARRAY_RFINDSTRUCT",
                0x2a => "ARRAY_ADD",
                0x2b => "ARRAY_INSERT",
                0x2c => "ARRAY_REMOVELAST",
                0x2d => "ARRAY_REMOVE",
                0x2e => "ARRAY_CLEAR",
                0x2f => "ARRAY_GETALLMATCHINGSTRUCTS",
                0x30 => "LOCK_GUARDS",
                0x31 => "UNLOCK_GUARDS",
                0x32 => "TRY_LOCK_GUARDS",
                _ => return Err(format!("unknown PEX opcode {op:#x}")),
            };
            Some(Stmt::ExprStmt {
                expr: name(format!(
                    "; {opcode_name} {}",
                    a.iter()
                        .map(|v| data_string(&v.data))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                pos: P,
            })
        }
    })
}

/// FO76 global functions whose signature is wider than FO4's. Keyed by the
/// FO76 arity so an already-FO4-shaped call is never trimmed twice.
fn fo4_static_args<'a>(class: &str, method: &str, raw: &'a [PexValuePayload]) -> &'a [PexValuePayload] {
    let class = class.to_lowercase();
    let method = method.to_lowercase();
    match (class.as_str(), method.as_str(), raw.len()) {
        // Trailing asDejaChannel.
        ("debug", "trace", 3) | ("debug", "traceuser", 4) => &raw[..raw.len() - 1],
        // Leading akReceivingPlayers: Player[].
        ("message", "clearhelpmessages", 1) => &raw[1..],
        _ => raw,
    }
}

fn structure_block(
    instructions: &[PexInstructionPayload],
    start: usize,
    end: usize,
    types: &HashMap<String, String>,
    compat: bool,
) -> Result<Vec<Stmt>, String> {
    if end > instructions.len() {
        return Err("PEX jump target is outside the function".into());
    }
    let mut result = Vec::new();
    let mut i = start;
    while i < end {
        let instr = &instructions[i];
        if matches!(instr.opcode, 0x15 | 0x16) {
            if instr.args.len() < 2 {
                return Err("PEX conditional jump has fewer than two arguments".into());
            }
            let target = i as i64 + instr.args[1].data.as_i64().unwrap_or_default();
            if target > i as i64 && target <= end as i64 {
                let target = target as usize;
                let pre = target - 1;
                let condition = if instr.opcode == 0x15 {
                    Expr::UnaryExpr { op: "!".into(), operand: Box::new(value_expr(&instr.args[0])), pos: P }
                } else { value_expr(&instr.args[0]) };
                if pre > i && instructions[pre].opcode == 0x14 {
                    let offset = instructions[pre]
                        .args
                        .first()
                        .and_then(|a| a.data.as_i64())
                        .ok_or("PEX JMP has no integer offset")?;
                    let jump_target = pre as i64 + offset;
                    if jump_target <= i as i64 {
                        let condition_start = usize::try_from(jump_target)
                            .ok()
                            .filter(|target| *target >= start)
                            .ok_or("PEX loop condition jumps outside its structured block")?;
                        let mut body = structure_block(instructions, i + 1, pre, types, compat)?;
                        // A back edge may target the instructions computing a compound
                        // condition, rather than the final conditional jump itself.
                        body.extend(structure_block(instructions, condition_start, i, types, compat)?);
                        result.push(Stmt::WhileStmt {
                            condition,
                            body,
                            pos: P,
                        });
                        i = target;
                        continue;
                    }
                    if jump_target > target as i64 {
                        result.push(Stmt::IfStmt {
                            condition,
                            body: structure_block(instructions, i + 1, pre, types, compat)?,
                            elseif_clauses: vec![],
                            else_body: structure_block(
                                instructions,
                                target,
                                jump_target as usize,
                                types,
                                compat,
                            )?,
                            pos: P,
                        });
                        i = jump_target as usize;
                        continue;
                    }
                }
                result.push(Stmt::IfStmt {
                    condition,
                    body: structure_block(instructions, i + 1, target, types, compat)?,
                    elseif_clauses: vec![],
                    else_body: vec![],
                    pos: P,
                });
                i = target;
                continue;
            }
        }
        if let Some(stmt) = instruction(instr, types, compat)? {
            result.push(stmt);
        }
        i += 1;
    }
    Ok(result)
}

fn visit_expr(expr: &Expr, visit: &mut impl FnMut(&Expr)) {
    visit(expr);
    match expr {
        Expr::DotExpr { object, .. } => visit_expr(object, visit),
        Expr::DotCallExpr { object, args, .. } => {
            visit_expr(object, visit);
            for arg in args {
                visit_expr(arg, visit);
            }
        }
        Expr::CallExpr { args, .. } => {
            for arg in args {
                visit_expr(arg, visit);
            }
        }
        Expr::BinaryExpr { left, right, .. } => {
            visit_expr(left, visit);
            visit_expr(right, visit);
        }
        Expr::UnaryExpr { operand, .. } => visit_expr(operand, visit),
        Expr::CastExpr { expr, .. } => visit_expr(expr, visit),
        Expr::ArrayAccessExpr { array, index, .. } => {
            visit_expr(array, visit);
            visit_expr(index, visit);
        }
        Expr::NewArrayExpr { size, .. } => visit_expr(size, visit),
        _ => {}
    }
}

fn visit_stmt(stmt: &Stmt, visit: &mut impl FnMut(&Expr)) {
    match stmt {
        Stmt::AssignStmt { target, value, .. } => {
            visit_expr(target, visit);
            visit_expr(value, visit);
        }
        Stmt::ExprStmt { expr, .. } => visit_expr(expr, visit),
        Stmt::ReturnStmt { value, .. } | Stmt::LocalVarStmt { value, .. } => {
            if let Some(expr) = value {
                visit_expr(expr, visit);
            }
        }
        Stmt::IfStmt {
            condition,
            body,
            elseif_clauses,
            else_body,
            ..
        } => {
            visit_expr(condition, visit);
            for stmt in body {
                visit_stmt(stmt, visit);
            }
            for clause in elseif_clauses {
                visit_expr(&clause.condition, visit);
                for stmt in &clause.body {
                    visit_stmt(stmt, visit);
                }
            }
            for stmt in else_body {
                visit_stmt(stmt, visit);
            }
        }
        Stmt::WhileStmt {
            condition, body, ..
        } => {
            visit_expr(condition, visit);
            for stmt in body {
                visit_stmt(stmt, visit);
            }
        }
    }
}

fn references(stmt: &Stmt, target: &str) -> bool {
    let mut found = false;
    visit_stmt(stmt, &mut |expr| {
        if let Expr::NameExpr { name, .. } = expr {
            found |= name.eq_ignore_ascii_case(target);
        }
    });
    found
}

fn map_expr(expr: &mut Expr, change: &mut impl FnMut(&mut Expr)) {
    match expr {
        Expr::DotExpr { object, .. } => map_expr(object, change),
        Expr::DotCallExpr { object, args, .. } => {
            map_expr(object, change);
            for arg in args {
                map_expr(arg, change);
            }
        }
        Expr::CallExpr { args, .. } => {
            for arg in args {
                map_expr(arg, change);
            }
        }
        Expr::BinaryExpr { left, right, .. } => {
            map_expr(left, change);
            map_expr(right, change);
        }
        Expr::UnaryExpr { operand, .. } => map_expr(operand, change),
        Expr::CastExpr { expr, .. } => map_expr(expr, change),
        Expr::ArrayAccessExpr { array, index, .. } => {
            map_expr(array, change);
            map_expr(index, change);
        }
        Expr::NewArrayExpr { size, .. } => map_expr(size, change),
        _ => {}
    }
    change(expr);
}

fn map_stmt(stmt: &mut Stmt, change: &mut impl FnMut(&mut Expr)) {
    match stmt {
        Stmt::AssignStmt { target, value, .. } => {
            map_expr(target, change);
            map_expr(value, change);
        }
        Stmt::ExprStmt { expr, .. } => map_expr(expr, change),
        Stmt::ReturnStmt { value, .. } | Stmt::LocalVarStmt { value, .. } => {
            if let Some(expr) = value {
                map_expr(expr, change);
            }
        }
        Stmt::IfStmt {
            condition,
            body,
            elseif_clauses,
            else_body,
            ..
        } => {
            map_expr(condition, change);
            for stmt in body {
                map_stmt(stmt, change);
            }
            for clause in elseif_clauses {
                map_expr(&mut clause.condition, change);
                for stmt in &mut clause.body {
                    map_stmt(stmt, change);
                }
            }
            for stmt in else_body {
                map_stmt(stmt, change);
            }
        }
        Stmt::WhileStmt {
            condition, body, ..
        } => {
            map_expr(condition, change);
            for stmt in body {
                map_stmt(stmt, change);
            }
        }
    }
}

fn substitute(expr: &mut Expr, target: &str, replacement: &Expr) {
    map_expr(expr, &mut |expr| {
        if let Expr::NameExpr { name, .. } = expr {
            if name == target {
                *expr = replacement.clone();
            }
        } else if let Expr::CastExpr {
            expr: inner,
            target_type,
            ..
        } = expr
        {
            if let Expr::CastExpr {
                target_type: inner_type,
                ..
            } = inner.as_ref()
            {
                if inner_type.eq_ignore_ascii_case(target_type) {
                    *expr = inner.as_ref().clone();
                }
            }
        }
    });
}

fn inline_temps(stmts: Vec<Stmt>, changed: &mut bool) -> Vec<Stmt> {
    let mut result = Vec::with_capacity(stmts.len());
    let mut iter = stmts.into_iter().peekable();
    while let Some(mut stmt) = iter.next() {
        match &mut stmt {
            Stmt::IfStmt {
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                *body = inline_temps(std::mem::take(body), changed);
                for clause in elseif_clauses {
                    clause.body = inline_temps(std::mem::take(&mut clause.body), changed);
                }
                *else_body = inline_temps(std::mem::take(else_body), changed);
            }
            Stmt::WhileStmt { body, .. } => *body = inline_temps(std::mem::take(body), changed),
            _ => {}
        }
        if let Stmt::AssignStmt {
            target: Expr::NameExpr { name, .. },
            value,
            ..
        } = &stmt
        {
            if is_temp(name) {
                if let Some(next) = iter.peek() {
                    let mut count = 0;
                    let mut counter = |expr: &Expr| {
                        if let Expr::NameExpr { name: used, .. } = expr {
                            count += usize::from(used == name);
                        }
                    };
                    visit_stmt(next, &mut counter);
                    let used_later = iter.clone().skip(1).any(|stmt| references(&stmt, name));
                    if count == 1 && !used_later {
                        let mut next = iter.next().unwrap();
                        match &mut next {
                            Stmt::IfStmt { condition, .. } | Stmt::WhileStmt { condition, .. } => {
                                substitute(condition, name, value)
                            }
                            Stmt::AssignStmt {
                                value: next_value, ..
                            } => substitute(next_value, name, value),
                            _ => map_stmt(&mut next, &mut |expr| {
                                if let Expr::NameExpr { name: used, .. } = expr {
                                    if used == name {
                                        *expr = value.clone();
                                    }
                                } else if let Expr::CastExpr {
                                    expr: inner,
                                    target_type,
                                    ..
                                } = expr
                                {
                                    if let Expr::CastExpr {
                                        target_type: inner_type,
                                        ..
                                    } = inner.as_ref()
                                    {
                                        if inner_type.eq_ignore_ascii_case(target_type) {
                                            *expr = inner.as_ref().clone();
                                        }
                                    }
                                }
                            }),
                        }
                        result.push(next);
                        *changed = true;
                        continue;
                    }
                }
            }
        }
        result.push(stmt);
    }
    result
}

fn drop_unused_call_results(stmts: &mut [Stmt]) {
    for stmt in stmts.iter_mut() {
        match stmt {
            Stmt::IfStmt {
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                drop_unused_call_results(body);
                for clause in elseif_clauses {
                    drop_unused_call_results(&mut clause.body);
                }
                drop_unused_call_results(else_body);
            }
            Stmt::WhileStmt { body, .. } => drop_unused_call_results(body),
            _ => {}
        }
    }
    for index in 0..stmts.len() {
        if let Stmt::AssignStmt {
            target: Expr::NameExpr { name, .. },
            value,
            pos,
            ..
        } = &stmts[index]
        {
            if is_temp(name)
                && matches!(value, Expr::CallExpr { .. } | Expr::DotCallExpr { .. })
                && !stmts[index + 1..].iter().any(|stmt| references(stmt, name))
            {
                stmts[index] = Stmt::ExprStmt {
                    expr: value.clone(),
                    pos: *pos,
                };
            }
        }
    }
}

fn cleanup_statements(stmts: &mut [Stmt], void_return: bool) {
    for stmt in stmts {
        match stmt {
            Stmt::AssignStmt {
                target: Expr::NameExpr { name, .. },
                value,
                pos,
                ..
            } if name.eq_ignore_ascii_case("::nonevar") => {
                *stmt = Stmt::ExprStmt {
                    expr: value.clone(),
                    pos: *pos,
                };
            }
            Stmt::ReturnStmt {
                value: Some(Expr::LiteralExpr { ty, .. }),
                pos,
            } if void_return && ty == "none" => {
                *stmt = Stmt::ReturnStmt {
                    value: None,
                    pos: *pos,
                };
            }
            Stmt::IfStmt {
                body,
                elseif_clauses,
                else_body,
                ..
            } => {
                cleanup_statements(body, void_return);
                for clause in elseif_clauses {
                    cleanup_statements(&mut clause.body, void_return);
                }
                cleanup_statements(else_body, void_return);
            }
            Stmt::WhileStmt { body, .. } => cleanup_statements(body, void_return),
            _ => {}
        }
    }
}

pub fn decompile_function(
    function: &PexFunctionPayload,
    auto_vars: &HashMap<String, String>,
    normalize: &impl Fn(&str) -> String,
    emit_locals: bool,
    external_types: &HashMap<String, String>,
    fo4_api_compat: bool,
) -> Result<Vec<Stmt>, String> {
    let mut types = external_types.clone();
    for local in &function.locals {
        types.insert(local.name.clone(), normalize(&local.ty));
    }
    for param in &function.params {
        types.insert(param.name.clone(), normalize(&param.ty));
    }
    let mut locals = Vec::new();
    let mut temp_declarations = HashSet::new();
    if emit_locals {
        let mut declared: HashSet<_> = function
            .params
            .iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        for local in &function.locals {
            let source = source_name(&local.name);
            let ty = normalize(&local.ty);
            if local.name.eq_ignore_ascii_case("::nonevar")
                || source.is_empty()
                || declared.contains(&source.to_lowercase())
                || ty.is_empty()
                || ty.eq_ignore_ascii_case("none")
            {
                continue;
            }
            locals.push(Stmt::LocalVarStmt {
                name: source.into(),
                ty,
                value: None,
                pos: P,
            });
            if local.name.to_lowercase().starts_with("::temp") {
                temp_declarations.insert(source.to_lowercase());
            }
            declared.insert(source.to_lowercase());
        }
    }
    let mut body = structure_block(
        &function.instructions,
        0,
        function.instructions.len(),
        &types,
        fo4_api_compat,
    )?;
    for _ in 0..10 {
        let mut changed = false;
        body = inline_temps(body, &mut changed);
        if !changed {
            break;
        }
    }
    drop_unused_call_results(&mut body);
    for stmt in &mut body {
        map_stmt(stmt, &mut |expr| {
            if let Expr::NameExpr { name, .. } = expr {
                if let Some(replacement) = auto_vars.get(name) {
                    *name = replacement.clone();
                } else if name.starts_with("::") && name.ends_with("_var") {
                    *name = name[2..name.len() - 4].to_owned();
                }
            }
        });
    }
    let void_return = matches!(function.return_type.as_str(), "None" | "NONE" | "none" | "");
    cleanup_statements(&mut body, void_return);
    if void_return && matches!(body.last(), Some(Stmt::ReturnStmt { value: None, .. })) {
        body.pop();
    }
    for stmt in &mut body {
        map_stmt(stmt, &mut |expr| {
            if let Expr::NameExpr { name, .. } = expr {
                *name = source_name(name).to_owned();
            }
        });
    }
    locals.retain(|stmt| match stmt {
        Stmt::LocalVarStmt { name, .. } => {
            !temp_declarations.contains(&name.to_lowercase())
                || body.iter().any(|stmt| references(stmt, name))
        }
        _ => true,
    });
    locals.extend(body);
    Ok(locals)
}

fn type_base(ty: &str) -> &str {
    ty.trim_end_matches("[]")
}

fn normalize_type(ty: &str, obj: &PexObjectPayload, options: &DecompileOptions) -> String {
    let base = type_base(ty);
    let suffix = &ty[base.len()..];
    let normalized = if let Some((script, member)) = base.split_once('#') {
        let member = obj
            .structs
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(member))
            .map(|s| s.name.as_str())
            .unwrap_or(member);
        if script.eq_ignore_ascii_case(&obj.name) {
            format!("{member}{suffix}")
        } else {
            format!("{script}:{member}{suffix}")
        }
    } else {
        ty.to_owned()
    };
    let base = type_base(&normalized);
    if obj
        .structs
        .iter()
        .any(|s| s.name.eq_ignore_ascii_case(base))
    {
        return normalized;
    }
    if let Some(alias) = options.type_aliases.get(&normalized.to_lowercase()) {
        return alias.clone();
    }
    options
        .type_aliases
        .get(&base.to_lowercase())
        .map(|alias| format!("{alias}{}", &normalized[base.len()..]))
        .unwrap_or(normalized)
}

pub fn type_names(pex: &PexFilePayload) -> Vec<String> {
    let Some(obj) = pex.objects.first() else {
        return vec![];
    };
    let mut result = HashSet::new();
    let options = DecompileOptions::default();
    let mut add = |ty: &str| {
        let normalized = normalize_type(ty, obj, &options);
        if !obj
            .structs
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case(type_base(&normalized)))
        {
            result.insert(normalized);
        }
    };
    add(&obj.parent);
    for var in &obj.variables {
        add(&var.ty);
    }
    for prop in &obj.properties {
        add(&prop.ty);
    }
    for st in &obj.structs {
        for member in &st.members {
            add(&member.ty);
        }
    }
    for function in obj.states.iter().flat_map(|s| &s.functions).chain(
        obj.properties
            .iter()
            .flat_map(|p| p.getter.iter().chain(&p.setter)),
    ) {
        add(&function.return_type);
        for param in &function.params {
            add(&param.ty);
        }
        for local in &function.locals {
            add(&local.ty);
        }
    }
    let mut result: Vec<_> = result.into_iter().collect();
    result.sort();
    result
}

fn flags(value: u32, pex: &PexFilePayload) -> Vec<String> {
    let mut result = Vec::new();
    let mut positions = HashMap::new();
    for flag in &pex.user_flags {
        if let Some(position) = positions.get(&flag.index) {
            result[*position] = flag.name.clone();
        } else if flag.index < 32 && value & (1 << flag.index) != 0 {
            positions.insert(flag.index, result.len());
            result.push(flag.name.clone());
        }
    }
    result
}

fn remote_event(function: &PexFunctionPayload) -> Option<String> {
    let raw = function.name.strip_prefix("::remote_")?;
    let sender = type_base(&function.params.first()?.ty);
    if !sender.is_empty()
        && raw
            .to_lowercase()
            .starts_with(&format!("{}_", sender.to_lowercase()))
    {
        Some(format!("{sender}.{}", &raw[sender.len() + 1..]))
    } else {
        None
    }
}

fn is_event(function: &PexFunctionPayload) -> bool {
    let lower = function.name.to_lowercase();
    matches!(
        lower.as_str(),
        "oninit"
            | "onactivate"
            | "onhit"
            | "ondying"
            | "ondeath"
            | "onload"
            | "onunload"
            | "oncellattach"
            | "oncelldetach"
            | "onreset"
            | "onopen"
            | "onclose"
            | "ontriggerenter"
            | "ontriggerleave"
            | "onequipped"
            | "onunequipped"
            | "oncontainerchanged"
            | "onitemadded"
            | "onitemremoved"
            | "onread"
            | "onsell"
            | "onworkshopobjectplaced"
            | "onworkshopobjectdestroyed"
            | "onworkshopobjectmoved"
            | "ontimer"
            | "onplayerloadgame"
            | "onbeginstate"
            | "onendstate"
            | "onquest init"
            | "onstagesetonquest"
            | "onaliaschanged"
            | "onaliasreset"
            | "onaliasshutdown"
            | "oneffectstart"
            | "oneffectfinish"
            | "onmagiceffectapply"
            | "onobjectequipped"
            | "onobjectunequipped"
            | "oncombatstatetchanged"
            | "onlocationchange"
            | "onpackagechange"
            | "onpackagestart"
            | "onpackageend"
    ) || (matches!(function.return_type.as_str(), "None" | "NONE" | "none" | "")
        && lower.starts_with("on"))
}

fn compat_local(param: Parameter) -> Stmt {
    let value = match param.ty.to_lowercase().as_str() {
        "bool" => literal(LiteralValue::Bool(false), "bool"),
        "int" => literal(LiteralValue::Int(0), "int"),
        "float" => literal(LiteralValue::Float(0.0), "float"),
        "string" => literal(LiteralValue::Str(String::new()), "string"),
        _ => literal(LiteralValue::Null, "none"),
    };
    Stmt::LocalVarStmt {
        name: param.name,
        ty: param.ty,
        value: Some(value),
        pos: P,
    }
}

fn adapt_event(parent: &str, event: &str, params: &mut Vec<Parameter>, body: &mut Vec<Stmt>) {
    let parent = parent.to_lowercase();
    let event = event.to_lowercase();
    let mut dropped = Vec::new();
    let ty = |index: usize| params[index].ty.to_lowercase();
    // Effect events keep the FO76 shape check but not a parent check: the
    // script may extend another ActiveMagicEffect script.
    let player_param = |index: usize| matches!(ty(index).as_str(), "actor" | "player");
    if event == "oneffectstart" && params.len() == 4 && ty(2) == "float" && ty(3) == "float" {
        dropped = params.split_off(2);
    } else if event == "oneffectfinish"
        && params.len() == 5
        && (2..5).all(|i| ty(i) == "float")
    {
        dropped = params.split_off(2);
    } else if event == "onmenuitemrun"
        && params.len() == 5
        && ty(0) == "int"
        && ty(1) == "terminal"
        && ty(2) == "objectreference"
        && ty(3) == "objectreference"
    {
        // FO4: OnMenuItemRun(Int auiMenuItemID, ObjectReference akTerminalRef).
        let quest = params.remove(4);
        let activator = params.remove(3);
        let base = params.remove(1);
        let terminal_ref = name(params[1].name.clone());
        let mut locals = Vec::new();
        for (param, value) in [
            (
                base,
                Some(Expr::CastExpr {
                    expr: Box::new(call(terminal_ref, "GetBaseObject", vec![])),
                    target_type: "Terminal".into(),
                    pos: P,
                }),
            ),
            (activator, Some(call(name("Game"), "GetPlayer", vec![]))),
            (quest, None),
        ] {
            if body.iter().any(|stmt| references(stmt, &param.name)) {
                locals.push(match value {
                    Some(value) => Stmt::LocalVarStmt {
                        name: param.name,
                        ty: param.ty,
                        value: Some(value),
                        pos: P,
                    },
                    None => compat_local(param),
                });
            }
        }
        locals.append(body);
        *body = locals;
    } else if (event == "onplayerteleport" && params.len() == 1 && player_param(0))
        || (matches!(
            event.as_str(),
            "onworkshopobjectgrabbed" | "onworkshopobjectmoved" | "onworkshopobjectplaced"
        ) && params.len() == 2
            && player_param(1))
    {
        let player = params.pop().expect("checked length");
        if body.iter().any(|stmt| references(stmt, &player.name)) {
            body.insert(
                0,
                Stmt::LocalVarStmt {
                    name: player.name,
                    ty: "Actor".into(),
                    value: Some(call(name("Game"), "GetPlayer", vec![])),
                    pos: P,
                },
            );
        }
    } else if event == "onhit" && params.len() == 10 && ty(8) == "bool" && ty(9) == "string" {
        dropped.push(params.remove(8));
    } else if event == "onradiationdamage"
        && params.len() == 3
        && ty(1) == "float"
        && ty(2) == "bool"
    {
        dropped.push(params.remove(1));
    } else if parent == "topicinfo"
        && matches!(event.as_str(), "onbegin" | "onend")
        && params.len() == 4
        && ty(0) == "objectreference"
        && ty(1) == "objectreference"
        && matches!(ty(2).as_str(), "quest" | "questinstance")
        && ty(3) == "bool"
    {
        let target = params.remove(1);
        let quest = params.remove(1);
        let mut locals = Vec::new();
        for (param, object, method) in [
            (target, "Game", "GetPlayer"),
            (quest, "Self", "GetOwningQuest"),
        ] {
            if body.iter().any(|stmt| references(stmt, &param.name)) {
                locals.push(Stmt::LocalVarStmt {
                    name: param.name,
                    ty: param.ty,
                    value: Some(call(name(object), method, vec![])),
                    pos: P,
                });
            }
        }
        locals.append(body);
        *body = locals;
    }
    if !dropped.is_empty() {
        let mut locals: Vec<_> = dropped
            .into_iter()
            .filter(|p| body.iter().any(|stmt| references(stmt, &p.name)))
            .map(compat_local)
            .collect();
        locals.append(body);
        *body = locals;
    }
}

fn function_def(
    function: &PexFunctionPayload,
    auto_vars: &HashMap<String, String>,
    normalize: &impl Fn(&str) -> String,
    members: &HashMap<String, String>,
    compat: bool,
) -> Result<FunctionDef, String> {
    Ok(FunctionDef {
        name: function.name.clone(),
        return_type: normalize(&function.return_type),
        params: function
            .params
            .iter()
            .map(|p| Parameter {
                name: p.name.clone(),
                ty: normalize(&p.ty),
                ..Default::default()
            })
            .collect(),
        is_native: function.is_native,
        is_global: function.is_global,
        docstring: function.docstring.clone(),
        body: decompile_function(function, auto_vars, normalize, true, members, compat)?,
        ..Default::default()
    })
}

pub fn decompile(pex: &PexFilePayload, options: &DecompileOptions) -> Result<ScriptNode, String> {
    let Some(obj) = pex.objects.first() else {
        return Ok(ScriptNode {
            name: "EmptyScript".into(),
            ..Default::default()
        });
    };
    let normalize = |ty: &str| normalize_type(ty, obj, options);
    let mut script_flags = flags(obj.user_flags, pex);
    if options.drop_script_const {
        script_flags.retain(|f| !f.eq_ignore_ascii_case("const"));
    } else if obj.is_const && !script_flags.iter().any(|f| f.eq_ignore_ascii_case("const")) {
        script_flags.push("Const".into());
    }
    let auto_vars: HashMap<_, _> = obj
        .properties
        .iter()
        .filter(|p| !p.auto_var.is_empty())
        .map(|p| (p.auto_var.clone(), p.name.clone()))
        .collect();
    let variables: HashMap<_, _> = obj.variables.iter().map(|v| (v.name.as_str(), v)).collect();
    let mut members = HashMap::new();
    for var in &obj.variables {
        members.insert(var.name.clone(), normalize(&var.ty));
    }
    for prop in &obj.properties {
        members.insert(prop.name.clone(), normalize(&prop.ty));
        if !prop.auto_var.is_empty() {
            members.insert(prop.auto_var.clone(), normalize(&prop.ty));
        }
    }
    let mut script = ScriptNode {
        name: obj.name.clone(),
        parent: (!obj.parent.is_empty()).then(|| normalize(&obj.parent)),
        flags: script_flags,
        variables: obj
            .variables
            .iter()
            .filter(|v| !auto_vars.contains_key(&v.name))
            .map(|v| VariableDef {
                name: v.name.clone(),
                ty: normalize(&v.ty),
                value: default_expr(&v.data),
                flags: flags(v.user_flags, pex),
                pos: P,
            })
            .collect(),
        ..Default::default()
    };
    for prop in &obj.properties {
        let mut prop_flags = Vec::new();
        if prop.flags & 4 != 0 {
            prop_flags.push(
                if prop.flags & 1 != 0 && prop.flags & 2 == 0 {
                    "AutoReadOnly"
                } else {
                    "Auto"
                }
                .into(),
            );
        }
        prop_flags.extend(flags(prop.user_flags, pex));
        if let Some(var) = variables.get(prop.auto_var.as_str()) {
            let already: HashSet<_> = prop_flags
                .iter()
                .map(|f: &String| f.to_lowercase())
                .collect();
            prop_flags.extend(flags(var.user_flags, pex).into_iter().filter(|f| {
                f.eq_ignore_ascii_case("conditional") && !already.contains(&f.to_lowercase())
            }));
        }
        let handler = |function: &PexFunctionPayload, name: &str| -> Result<FunctionDef, String> {
            let mut result = function_def(
                function,
                &auto_vars,
                &normalize,
                &members,
                options.fo4_api_compat,
            )?;
            result.name = name.into();
            Ok(result)
        };
        script.properties.push(PropertyDef {
            name: prop.name.clone(),
            ty: normalize(&prop.ty),
            flags: prop_flags,
            docstring: prop.docstring.clone(),
            default: variables
                .get(prop.auto_var.as_str())
                .and_then(|v| default_expr(&v.data)),
            getter: prop
                .getter
                .as_ref()
                .map(|f| handler(f, "Get"))
                .transpose()?,
            setter: prop
                .setter
                .as_ref()
                .map(|f| handler(f, "Set"))
                .transpose()?,
            pos: P,
        });
    }
    for st in &obj.structs {
        script.structs.push(StructDef {
            name: st.name.clone(),
            members: st
                .members
                .iter()
                .map(|member| {
                    let mut member_flags = flags(member.user_flags, pex);
                    if member.is_const {
                        member_flags.push("Const".into());
                    }
                    StructMemberDef {
                        name: member.name.clone(),
                        ty: normalize(&member.ty),
                        value: default_expr(&member.data),
                        flags: member_flags,
                        pos: P,
                    }
                })
                .collect(),
            pos: P,
        });
    }
    for state in &obj.states {
        let mut converted = StateDef {
            name: state.name.clone(),
            is_auto: state.name == obj.auto_state,
            ..Default::default()
        };
        for function in &state.functions {
            let remote = remote_event(function);
            if options.skip_internal_functions
                && function.name.starts_with("::")
                && remote.is_none()
            {
                continue;
            }
            let event = is_event(function);
            let mut converted_fn = function_def(
                function,
                &auto_vars,
                &normalize,
                &members,
                options.fo4_api_compat,
            )?;
            if options.fo4_api_compat && event {
                adapt_event(
                    &obj.parent,
                    &function.name,
                    &mut converted_fn.params,
                    &mut converted_fn.body,
                );
            }
            if remote.is_some() || event {
                converted.events.push(EventDef {
                    name: remote.unwrap_or(converted_fn.name),
                    params: converted_fn.params,
                    is_native: converted_fn.is_native,
                    docstring: converted_fn.docstring,
                    body: converted_fn.body,
                    pos: P,
                });
            } else {
                converted.functions.push(converted_fn);
            }
        }
        if state.name.is_empty() {
            script.functions.extend(converted.functions);
            script.events.extend(converted.events);
        } else {
            script.states.push(converted);
        }
    }
    restore_custom_events(&mut script, &members);
    Ok(script)
}

fn sender_type<'a>(expr: &Expr, members: &'a HashMap<String, String>) -> &'a str {
    match expr {
        Expr::CastExpr { expr, .. } => sender_type(expr, members),
        Expr::NameExpr { name, .. } => type_base(
            members
                .get(&name.to_lowercase())
                .map(String::as_str)
                .unwrap_or_default(),
        ),
        _ => "",
    }
}

fn strip_event_prefix(expr: &mut Expr, owner: &str) {
    if owner.is_empty() {
        return;
    }
    if let Expr::LiteralExpr {
        value: LiteralValue::Str(value),
        ..
    } = expr
    {
        if value
            .to_lowercase()
            .starts_with(&format!("{}_", owner.to_lowercase()))
        {
            *value = value[owner.len() + 1..].to_owned();
        }
    }
}

fn restore_custom_events(script: &mut ScriptNode, members: &HashMap<String, String>) {
    let members = members
        .iter()
        .map(|(key, val)| (key.to_lowercase(), val.clone()))
        .collect();
    let script_name = &script.name;
    let mut restore = |expr: &mut Expr| {
        if let Expr::DotCallExpr { method, args, .. } = expr {
            if method.eq_ignore_ascii_case("sendcustomevent") && !args.is_empty() {
                strip_event_prefix(&mut args[0], script_name);
            } else if matches!(
                method.to_lowercase().as_str(),
                "registerforcustomevent" | "unregisterforcustomevent"
            ) && args.len() >= 2
            {
                let sender = sender_type(&args[0], &members);
                strip_event_prefix(&mut args[1], sender);
                if let Expr::CastExpr {
                    expr, target_type, ..
                } = &args[0]
                {
                    if target_type.eq_ignore_ascii_case("scriptobject") {
                        args[0] = expr.as_ref().clone();
                    }
                }
            }
        }
    };
    for body in script
        .functions
        .iter_mut()
        .map(|f| &mut f.body)
        .chain(script.events.iter_mut().map(|f| &mut f.body))
        .chain(script.states.iter_mut().flat_map(|s| {
            s.functions
                .iter_mut()
                .map(|f| &mut f.body)
                .chain(s.events.iter_mut().map(|e| &mut e.body))
        }))
    {
        for stmt in body {
            map_stmt(stmt, &mut restore);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod loop_conditions;

    #[test]
    fn compiled_pex_matches_python_decompiler() {
        let expected: HashMap<String, String> =
            serde_json::from_str(include_str!("../tests/decompile_golden.json")).unwrap();
        for (file, expected) in expected {
            let path = format!("{}/tests/codegen_golden/{file}", env!("CARGO_MANIFEST_DIR"));
            let pex = parse_pex_file(&path).unwrap();
            let script = decompile(&pex, &DecompileOptions::default()).unwrap();
            assert_eq!(crate::emitter::emit_script(&script), expected, "{file}");
        }
    }

    #[test]
    fn synthetic_cases_match_python_decompiler() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/decompile_cases.json")).unwrap();
        for (index, case) in cases.iter().enumerate() {
            if case["kind"] == "script" {
                let pex = serde_json::from_value(case["pex"].clone()).unwrap();
                let options = serde_json::from_value(case["options"].clone()).unwrap();
                let script = decompile(&pex, &options).unwrap();
                assert_eq!(
                    crate::emitter::emit_script(&script),
                    case["expected"].as_str().unwrap(),
                    "case {index}"
                );
            } else {
                let function = serde_json::from_value(case["function"].clone()).unwrap();
                let body = decompile_function(
                    &function,
                    &HashMap::new(),
                    &str::to_owned,
                    false,
                    &HashMap::new(),
                    false,
                )
                .unwrap();
                let mut actual = serde_json::to_value(body).unwrap();
                remove_arg_names(&mut actual);
                assert_eq!(actual, case["expected"], "case {index}");
            }
        }
    }

    #[test]
    fn float_spellings_compile_to_identical_pex() {
        for (previous, native) in [
            ("0.4600000083446502", "0.46000000834465027"),
            ("0.09600000083446504", "0.09600000083446503"),
            ("0.029999999329447743", "0.029999999329447746"),
            ("0.119999997317791", "0.11999999731779099"),
            ("99.6999969482422", "99.69999694824219"),
        ] {
            let compile = |value| {
                let source = format!(
                    "Scriptname FloatPrecision\nFloat Function Value()\nReturn {value}\nEndFunction\n"
                );
                let result =
                    crate::compiler::compile_source(&source, &[], crate::profile::Game::Fo4, None);
                assert!(result.ok, "{:?}", result.diagnostics);
                result.pex_bytes.unwrap()
            };
            assert_eq!(compile(previous), compile(native), "{previous} / {native}");
        }
    }

    #[test]
    fn malformed_instruction_arguments_return_errors() {
        let types = HashMap::new();
        for opcode in [1, 0x0a, 0x17, 0x18, 0x19, 0x1e, 0x25] {
            assert!(
                instruction(
                    &PexInstructionPayload {
                        opcode,
                        args: vec![]
                    },
                    &types,
                    false
                )
                .is_err()
            );
        }
        assert!(
            instruction(
                &PexInstructionPayload {
                    opcode: 0xff,
                    args: vec![]
                },
                &types,
                false
            )
            .is_err()
        );
        assert!(
            structure_block(
                &[PexInstructionPayload {
                    opcode: 0x16,
                    args: vec![]
                }],
                0,
                1,
                &types,
                false
            )
            .is_err()
        );
    }

    fn remove_arg_names(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.remove("arg_names");
                for child in map.values_mut() {
                    remove_arg_names(child);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    remove_arg_names(child);
                }
            }
            _ => {}
        }
    }
}
