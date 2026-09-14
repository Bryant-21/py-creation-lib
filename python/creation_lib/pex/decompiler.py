"""PEX bytecode decompiler.

Transforms PexFile structures into ast_nodes for source emission.
"""
from __future__ import annotations
from typing import Callable

from creation_lib.pex.types import (
    PexFile, PexObject, PexState, PexFunction, PexInstruction,
    PexValue, PexVariable, PexProperty, PexParam, PexLocal, ValueType,
)
from creation_lib.pex.opcodes import PexOpcode
from creation_lib.papyrus_lsp.ast_nodes import (
    Pos, NameExpr, LiteralExpr, BinaryExpr, UnaryExpr, CastExpr,
    DotExpr, CallExpr, DotCallExpr, ArrayAccessExpr, NewArrayExpr,
    ParentExpr, ExprStmt, AssignStmt, ReturnStmt, IfStmt, WhileStmt,
    LocalVarStmt, FunctionDef, EventDef, PropertyDef, VariableDef,
    StateDef, StructDef, StructMemberDef, ScriptNode, Parameter,
)

P = Pos(0, 0, 0, 0)
TypeAdapter = Callable[[str], str]


def _compat_local_for_dropped_param(param: Parameter) -> LocalVarStmt:
    type_name = param.type.lower()
    if type_name == "bool":
        initial = LiteralExpr(False, "bool", P)
    elif type_name == "int":
        initial = LiteralExpr(0, "int", P)
    elif type_name == "float":
        initial = LiteralExpr(0.0, "float", P)
    elif type_name == "string":
        initial = LiteralExpr("", "string", P)
    else:
        initial = LiteralExpr(None, "none", P)
    return LocalVarStmt(param.name, param.type, initial, P)

# Opcode → binary operator string
_BINARY_OPS = {
    PexOpcode.IADD: "+", PexOpcode.FADD: "+",
    PexOpcode.ISUB: "-", PexOpcode.FSUB: "-",
    PexOpcode.IMUL: "*", PexOpcode.FMUL: "*",
    PexOpcode.IDIV: "/", PexOpcode.FDIV: "/",
    PexOpcode.IMOD: "%",
    PexOpcode.CMP_EQ: "==", PexOpcode.CMP_LT: "<",
    PexOpcode.CMP_LTE: "<=", PexOpcode.CMP_GT: ">",
    PexOpcode.CMP_GTE: ">=",
    PexOpcode.STRCAT: "+",
}

# Known event names (used to distinguish events from functions)
_EVENT_NAMES = {
    "oninit", "onactivate", "onhit", "ondying", "ondeath", "onload",
    "onunload", "oncellattach", "oncelldetach", "onreset", "onopen",
    "onclose", "ontriggerenter", "ontriggerleave", "onequipped",
    "onunequipped", "oncontainerchanged", "onitemadded", "onitemremoved",
    "onread", "onsell", "onworkshopobjectplaced", "onworkshopobjectdestroyed",
    "onworkshopobjectmoved", "ontimer", "onplayerloadgame",
    "onbeginstate", "onendstate", "onquest init", "onstagesetonquest",
    "onaliaschanged", "onaliasreset", "onaliasshutdown",
    "oneffectstart", "oneffectfinish", "onmagiceffectapply",
    "onobjectequipped", "onobjectunequipped", "oncombatstatetchanged",
    "onlocationchange", "onpackagechange", "onpackagestart", "onpackageend",
}


def _value_to_expr(val: PexValue) -> object:
    """Convert a PexValue to an AST expression node."""
    if val.type == ValueType.NONE:
        return LiteralExpr(None, "none", P)
    elif val.type == ValueType.IDENTIFIER:
        name = val.data
        if name == "self":
            return NameExpr("Self", P)
        if name == "::NoneVar":
            return LiteralExpr(None, "none", P)
        return NameExpr(str(name), P)
    elif val.type == ValueType.STRING:
        return LiteralExpr(str(val.data), "string", P)
    elif val.type == ValueType.INTEGER:
        return LiteralExpr(int(val.data), "int", P)
    elif val.type == ValueType.FLOAT:
        return LiteralExpr(float(val.data), "float", P)
    elif val.type == ValueType.BOOL:
        return LiteralExpr(bool(val.data), "bool", P)
    return NameExpr(str(val.data), P)


def _is_temp(name: str) -> bool:
    """Check if a variable name is a compiler-generated temporary."""
    return name.startswith("::temp") or name.startswith("::NoneVar") or name.startswith("__temp")


def _source_identifier_name(name: str) -> str:
    if name.startswith("::temp"):
        return name[2:]
    if name.startswith("::"):
        return name[2:]
    return name


def _is_none_var(val: PexValue) -> bool:
    """Check if a value is the ::NoneVar placeholder (void return)."""
    return val.type == ValueType.IDENTIFIER and val.data == "::NoneVar"


def _is_empty_string_value(val: PexValue) -> bool:
    return val.type == ValueType.STRING and val.data == ""


def _node_references_name(node: object, name: str) -> bool:
    if isinstance(node, NameExpr):
        return node.name.lower() == name.lower()
    if isinstance(node, AssignStmt):
        return (
            _node_references_name(node.target, name)
            or _node_references_name(node.value, name)
        )
    if isinstance(node, BinaryExpr):
        return (
            _node_references_name(node.left, name)
            or _node_references_name(node.right, name)
        )
    if isinstance(node, UnaryExpr):
        return _node_references_name(node.operand, name)
    if isinstance(node, CastExpr):
        return _node_references_name(node.expr, name)
    if isinstance(node, DotExpr):
        return _node_references_name(node.object, name)
    if isinstance(node, DotCallExpr):
        return (
            _node_references_name(node.object, name)
            or any(_node_references_name(arg, name) for arg in node.args)
        )
    if isinstance(node, CallExpr):
        return any(_node_references_name(arg, name) for arg in node.args)
    if isinstance(node, ArrayAccessExpr):
        return (
            _node_references_name(node.array, name)
            or _node_references_name(node.index, name)
        )
    if isinstance(node, NewArrayExpr):
        return _node_references_name(node.size, name)
    if isinstance(node, ReturnStmt):
        return bool(node.value and _node_references_name(node.value, name))
    if isinstance(node, LocalVarStmt):
        return bool(node.value and _node_references_name(node.value, name))
    if isinstance(node, ExprStmt):
        return _node_references_name(node.expr, name)
    if isinstance(node, IfStmt):
        return (
            _node_references_name(node.condition, name)
            or any(_node_references_name(stmt, name) for stmt in node.body)
            or any(
                _node_references_name(cond, name)
                or any(_node_references_name(stmt, name) for stmt in body)
                for cond, body in node.elseif_clauses
            )
            or any(_node_references_name(stmt, name) for stmt in node.else_body)
        )
    if isinstance(node, WhileStmt):
        return (
            _node_references_name(node.condition, name)
            or any(_node_references_name(stmt, name) for stmt in node.body)
        )
    return False


def decompile_function(
    fn: PexFunction,
    auto_var_map: dict[str, str] | None = None,
    type_normalizer: object | None = None,
    emit_locals: bool = False,
    external_types: dict[str, str] | None = None,
    fo4_api_compat: bool = False,
) -> list:
    """Decompile a PexFunction's instructions into AST statement nodes.

    Args:
        fn: The PEX function to decompile.
        auto_var_map: Mapping of auto-property backing var names (e.g. "::MyProp_var")
                      to property names (e.g. "MyProp"). Passed from _decompile_object.

    Returns a list of statement AST nodes.
    """
    normalize_type = type_normalizer if callable(type_normalizer) else lambda t: t

    # Build local type map for CAST resolution
    local_types = dict(external_types or {})
    local_types.update({loc.name: normalize_type(loc.type) for loc in fn.locals})
    for param in fn.params:
        local_types[param.name] = normalize_type(param.type)

    local_decls = []
    compiler_temp_decl_names: set[str] = set()
    if emit_locals:
        declared = {p.name.lower() for p in fn.params}
        for loc in fn.locals:
            raw_name = str(loc.name or "")
            if raw_name.lower() == "::nonevar":
                continue
            source_name = _source_identifier_name(raw_name)
            if not source_name or source_name.lower() in declared:
                continue
            source_type = normalize_type(loc.type)
            if source_type.lower() in {"", "none"}:
                continue
            local_decls.append(LocalVarStmt(source_name, source_type, None, P))
            if raw_name.lower().startswith("::temp"):
                compiler_temp_decl_names.add(source_name.lower())
            declared.add(source_name.lower())

    # Recover control flow directly from instructions
    stmts = _structure_block(
        fn.instructions,
        0,
        len(fn.instructions),
        local_types,
        fo4_api_compat=fo4_api_compat,
    )

    # Inline temporaries (iterative — run until stable)
    for _ in range(10):
        new_stmts = _inline_temps(stmts)
        if new_stmts == stmts:
            break
        stmts = new_stmts
    stmts = _drop_unused_temp_call_results(stmts)

    # Rewrite auto-property backing vars to property names
    stmts = [_rewrite_auto_vars(s, auto_var_map or {}) for s in stmts]

    # Convert ::nonevar assignments to expression statements
    stmts = _cleanup_nonevar(stmts)

    # Remove trailing Return None for void functions
    if fn.return_type in ("None", "NONE", "none", "") and stmts:
        last = stmts[-1]
        if isinstance(last, ReturnStmt) and (last.value is None or (
            isinstance(last.value, LiteralExpr) and last.value.type == "none")):
            stmts = stmts[:-1]

    stmts = [_sanitize_internal_names(s) for s in stmts]
    if compiler_temp_decl_names:
        local_decls = [
            decl for decl in local_decls
            if decl.name.lower() not in compiler_temp_decl_names
            or any(_node_references_name(stmt, decl.name) for stmt in stmts)
        ]

    return local_decls + stmts


def _decompile_instruction(
    instr: PexInstruction,
    local_types: dict,
    idx: int,
    all_instrs: list,
    *,
    fo4_api_compat: bool = False,
) -> object | None:
    """Convert a single PEX instruction to an AST statement (or None for jumps handled in control flow)."""
    op = instr.opcode
    args = instr.args

    if op == PexOpcode.NOP:
        return None

    # Binary operations: dest = left OP right
    if op in _BINARY_OPS:
        dest = _value_to_expr(args[0])
        left = _value_to_expr(args[1])
        right = _value_to_expr(args[2])
        return AssignStmt(dest, "=", BinaryExpr(left, _BINARY_OPS[op], right, P), P)

    # Unary operations
    if op == PexOpcode.NOT:
        return AssignStmt(_value_to_expr(args[0]), "=", UnaryExpr("!", _value_to_expr(args[1]), P), P)
    if op == PexOpcode.INEG:
        return AssignStmt(_value_to_expr(args[0]), "=", UnaryExpr("-", _value_to_expr(args[1]), P), P)
    if op == PexOpcode.FNEG:
        return AssignStmt(_value_to_expr(args[0]), "=", UnaryExpr("-", _value_to_expr(args[1]), P), P)

    # Assignment
    if op == PexOpcode.ASSIGN:
        return AssignStmt(_value_to_expr(args[0]), "=", _value_to_expr(args[1]), P)

    # Cast
    if op == PexOpcode.CAST:
        dest_name = args[0].data if args[0].type == ValueType.IDENTIFIER else str(args[0].data)
        dest_type = local_types.get(dest_name, "var")
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            CastExpr(_value_to_expr(args[1]), dest_type, P), P,
        )

    # IS (isinstance)
    if op == PexOpcode.IS:
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            BinaryExpr(_value_to_expr(args[1]), "is", _value_to_expr(args[2]), P), P,
        )

    # Method call: CALLMETHOD method, obj, dest, argcount, args...
    if op == PexOpcode.CALLMETHOD:
        method = args[0].data
        obj_value = args[1]
        obj = _value_to_expr(obj_value)
        dest = args[2]
        # args[3] = arg count, args[4:] = actual args
        raw_call_args = args[4:]
        obj_type = ""
        if obj_value.type == ValueType.IDENTIFIER:
            obj_type = local_types.get(str(obj_value.data), "")
        if (
            fo4_api_compat
            and (not obj_type or obj_type.lower() == "sound")
            and str(method).lower() in {"play", "playandwait"}
            and raw_call_args
            and _is_empty_string_value(raw_call_args[-1])
        ):
            raw_call_args = raw_call_args[:-1]
        if (
            fo4_api_compat
            and obj_type.lower() == "weapon"
            and str(method).lower() == "fire"
            and len(raw_call_args) == 3
            and raw_call_args[-1].type == ValueType.BOOL
        ):
            raw_call_args = raw_call_args[:-1]
        # FO4 is GetRefsLinkedToMe(Keyword apLinkKeyword, Keyword apExcludeKeyword);
        # FO76 takes a third bool. The native compiler does not validate arity, so
        # keeping it ships a call the runtime rejects rather than a build error.
        if (
            fo4_api_compat
            and (not obj_type or obj_type.lower() in {"objectreference", "actor"})
            and str(method).lower() == "getrefslinkedtome"
            and len(raw_call_args) == 3
            and raw_call_args[-1].type == ValueType.BOOL
        ):
            raw_call_args = raw_call_args[:-1]
        # FO76 inserts a critical-hit filter before abMatch; FO4 has no
        # corresponding hit-event field and rejects the ten-argument call.
        if (
            fo4_api_compat
            and str(method).lower()
            in {"registerforhitevent", "unregisterforhitevent"}
            and len(raw_call_args) == 10
            and raw_call_args[8].type == ValueType.INTEGER
            and raw_call_args[9].type == ValueType.BOOL
        ):
            raw_call_args = raw_call_args[:8] + raw_call_args[9:]
        if (
            fo4_api_compat
            and (not obj_type or obj_type.lower() == "actor")
            and str(method).lower() == "setequippedweaponattacksenabled"
        ):
            return None
        if (
            fo4_api_compat
            and str(method).lower() in {
                "registerfordamagedealtevent",
                "unregisterfordamagedealtevent",
                "unregisterforalldamagedealtevents",
            }
        ):
            return None
        call_args = [_value_to_expr(a) for a in raw_call_args]
        call_expr = DotCallExpr(obj, str(method), call_args, P)
        if _is_none_var(dest):
            return ExprStmt(call_expr, P)
        return AssignStmt(_value_to_expr(dest), "=", call_expr, P)

    # Parent call: CALLPARENT method, dest, argcount, args...
    if op == PexOpcode.CALLPARENT:
        method = args[0].data
        dest = args[1]
        call_args = [_value_to_expr(a) for a in args[3:]]
        call_expr = DotCallExpr(ParentExpr(P), str(method), call_args, P)
        if _is_none_var(dest):
            return ExprStmt(call_expr, P)
        return AssignStmt(_value_to_expr(dest), "=", call_expr, P)

    # Static call: CALLSTATIC class, method, dest, argcount, args...
    if op == PexOpcode.CALLSTATIC:
        cls = args[0].data
        method = args[1].data
        dest = args[2]
        raw_call_args = args[4:]
        if (
            fo4_api_compat
            and str(cls).lower() == "game"
            and str(method).lower() == "getlocalplayer"
        ):
            method = "GetPlayer"
        if (
            fo4_api_compat
            and str(cls).lower() == "debug"
            and str(method).lower() == "trace"
            and len(raw_call_args) == 3
            and raw_call_args[-1].type == ValueType.STRING
        ):
            raw_call_args = raw_call_args[:-1]
        call_args = [_value_to_expr(a) for a in raw_call_args]
        call_expr = DotCallExpr(NameExpr(str(cls), P), str(method), call_args, P)
        if _is_none_var(dest):
            return ExprStmt(call_expr, P)
        return AssignStmt(_value_to_expr(dest), "=", call_expr, P)

    # Return
    if op == PexOpcode.RETURN:
        if not args or _is_none_var(args[0]):
            return ReturnStmt(None, P)
        return ReturnStmt(_value_to_expr(args[0]), P)

    # Property get: PROPGET prop, obj, dest
    if op == PexOpcode.PROPGET:
        prop = args[0].data
        obj = _value_to_expr(args[1])
        return AssignStmt(_value_to_expr(args[2]), "=", DotExpr(obj, str(prop), P), P)

    # Property set: PROPSET prop, obj, val
    if op == PexOpcode.PROPSET:
        prop = args[0].data
        obj = _value_to_expr(args[1])
        return AssignStmt(DotExpr(obj, str(prop), P), "=", _value_to_expr(args[2]), P)

    # Array operations
    if op == PexOpcode.ARRAY_CREATE:
        element_type = local_types.get(args[0].data, "var")
        if element_type.endswith("[]"):
            element_type = element_type[:-2]
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            NewArrayExpr(element_type, _value_to_expr(args[1]), P), P,
        )
    if op == PexOpcode.ARRAY_LENGTH:
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            DotExpr(_value_to_expr(args[1]), "Length", P), P,
        )
    if op == PexOpcode.ARRAY_GETELEMENT:
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            ArrayAccessExpr(_value_to_expr(args[1]), _value_to_expr(args[2]), P), P,
        )
    if op == PexOpcode.ARRAY_SETELEMENT:
        return AssignStmt(
            ArrayAccessExpr(_value_to_expr(args[0]), _value_to_expr(args[1]), P),
            "=", _value_to_expr(args[2]), P,
        )

    if op == PexOpcode.STRUCT_CREATE:
        dest_name = args[0].data if args[0].type == ValueType.IDENTIFIER else str(args[0].data)
        dest_type = local_types.get(dest_name, "var")
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            CallExpr(f"new {dest_type}", [], P), P,
        )
    if op == PexOpcode.STRUCT_GET:
        return AssignStmt(
            _value_to_expr(args[0]), "=",
            DotExpr(_value_to_expr(args[1]), str(args[2].data), P), P,
        )
    if op == PexOpcode.STRUCT_SET:
        return AssignStmt(
            DotExpr(_value_to_expr(args[0]), str(args[1].data), P),
            "=", _value_to_expr(args[2]), P,
        )

    # Jump instructions — handled in control flow recovery, emit as markers
    if op in (PexOpcode.JMP, PexOpcode.JMPT, PexOpcode.JMPF):
        return None  # handled by _structure_block

    # Fallback: emit as comment
    arg_strs = ", ".join(str(a.data) for a in args)
    return ExprStmt(NameExpr(f"; {op.name} {arg_strs}", P), P)


def _structure_block(
    instructions: list[PexInstruction],
    start: int,
    end: int,
    local_types: dict,
    *,
    fo4_api_compat: bool = False,
) -> list:
    """Recursively structure a block of instructions into AST statements."""
    stmts = []
    i = start
    while i < end:
        instr = instructions[i]

        if instr.opcode == PexOpcode.JMPF:
            # Potential If or While header
            cond_expr = _value_to_expr(instr.args[0])
            offset = instr.args[1].data if isinstance(instr.args[1].data, int) else 0
            target = i + offset

            # Check for While: does the block end with a backward JMP to i?
            if target <= end and target > i:
                # Look for backward jump at target-1
                pre_target = target - 1
                if (pre_target > i and pre_target < len(instructions) and
                    instructions[pre_target].opcode == PexOpcode.JMP):
                    jmp_offset = instructions[pre_target].args[0].data
                    jmp_target = pre_target + jmp_offset
                    if jmp_target <= i:
                        # While loop: condition at jmp_target..i, body at i+1..pre_target
                        body = _structure_block(
                            instructions,
                            i + 1,
                            pre_target,
                            local_types,
                            fo4_api_compat=fo4_api_compat,
                        )
                        stmts.append(WhileStmt(cond_expr, body, P))
                        i = target
                        continue

                # If/Else: check if there's a JMP before the target (else branch)
                pre_else = target - 1
                if (pre_else > i and pre_else < len(instructions) and
                    instructions[pre_else].opcode == PexOpcode.JMP):
                    jmp_offset = instructions[pre_else].args[0].data
                    else_end = pre_else + jmp_offset
                    if else_end > target:
                        # If/Else
                        if_body = _structure_block(
                            instructions,
                            i + 1,
                            pre_else,
                            local_types,
                            fo4_api_compat=fo4_api_compat,
                        )
                        else_body = _structure_block(
                            instructions,
                            target,
                            else_end,
                            local_types,
                            fo4_api_compat=fo4_api_compat,
                        )
                        stmts.append(IfStmt(cond_expr, if_body, [], else_body, P))
                        i = else_end
                        continue

                # If without else
                if_body = _structure_block(
                    instructions,
                    i + 1,
                    target,
                    local_types,
                    fo4_api_compat=fo4_api_compat,
                )
                stmts.append(IfStmt(cond_expr, if_body, [], [], P))
                i = target
                continue

        # Regular instruction
        if instr.opcode in (PexOpcode.JMP, PexOpcode.JMPT):
            i += 1
            continue  # skip orphan jumps

        stmt = _decompile_instruction(
            instr,
            local_types,
            i,
            instructions,
            fo4_api_compat=fo4_api_compat,
        )
        if stmt is not None:
            stmts.append(stmt)
        i += 1

    return stmts


def _inline_temps(stmts: list) -> list:
    """Inline compiler-generated temporaries that have a single definition
    immediately followed by a single use in the next statement.

    Works on a flat list of statements. For control flow blocks (If/While),
    recurses into their bodies.
    """
    result = []
    i = 0
    while i < len(stmts):
        stmt = stmts[i]

        # Recurse into control flow bodies
        if isinstance(stmt, IfStmt):
            new_body = _inline_temps(stmt.body)
            new_elseifs = [(c, _inline_temps(b)) for c, b in stmt.elseif_clauses]
            new_else = _inline_temps(stmt.else_body)
            stmt = IfStmt(stmt.condition, new_body, new_elseifs, new_else, stmt.pos)
        elif isinstance(stmt, WhileStmt):
            new_body = _inline_temps(stmt.body)
            stmt = WhileStmt(stmt.condition, new_body, stmt.pos)

        # Check if this is a temp assignment that can be inlined into the next stmt
        if (i + 1 < len(stmts)
            and isinstance(stmt, AssignStmt)
            and isinstance(stmt.target, NameExpr)
            and _is_temp(stmt.target.name)):

            temp_name = stmt.target.name
            next_stmt = stmts[i + 1]

            # Count uses of this temp in the next statement (condition only for If/While)
            uses = {}
            if isinstance(next_stmt, (IfStmt, WhileStmt)):
                # Only count uses in the condition, not the body
                _count_uses(next_stmt.condition, uses)
            elif isinstance(next_stmt, AssignStmt):
                _count_uses(next_stmt.value, uses)
            else:
                _count_uses(next_stmt, uses)
            use_count = uses.get(temp_name, 0)

            if use_count == 1:
                # Inline: substitute temp in next statement's condition/expression only
                inline_map = {temp_name: stmt.value}
                if isinstance(next_stmt, IfStmt):
                    new_cond = _substitute_temps(next_stmt.condition, inline_map)
                    result.append(IfStmt(new_cond, next_stmt.body, next_stmt.elseif_clauses,
                                         next_stmt.else_body, next_stmt.pos))
                elif isinstance(next_stmt, WhileStmt):
                    new_cond = _substitute_temps(next_stmt.condition, inline_map)
                    result.append(WhileStmt(new_cond, next_stmt.body, next_stmt.pos))
                elif isinstance(next_stmt, AssignStmt):
                    result.append(AssignStmt(
                        next_stmt.target,
                        next_stmt.op,
                        _substitute_temps(next_stmt.value, inline_map),
                        next_stmt.pos,
                    ))
                else:
                    result.append(_substitute_temps(next_stmt, inline_map))
                i += 2
                continue

        result.append(stmt)
        i += 1

    return result


def _drop_unused_temp_call_results(stmts: list) -> list:
    scoped = []
    for stmt in stmts:
        if isinstance(stmt, IfStmt):
            scoped.append(IfStmt(
                stmt.condition,
                _drop_unused_temp_call_results(stmt.body),
                [
                    (cond, _drop_unused_temp_call_results(body))
                    for cond, body in stmt.elseif_clauses
                ],
                _drop_unused_temp_call_results(stmt.else_body),
                stmt.pos,
            ))
        elif isinstance(stmt, WhileStmt):
            scoped.append(WhileStmt(
                stmt.condition,
                _drop_unused_temp_call_results(stmt.body),
                stmt.pos,
            ))
        else:
            scoped.append(stmt)

    result = []
    for index, stmt in enumerate(scoped):
        if (
            isinstance(stmt, AssignStmt)
            and isinstance(stmt.target, NameExpr)
            and _is_temp(stmt.target.name)
            and isinstance(stmt.value, (CallExpr, DotCallExpr))
            and not any(
                _node_references_name(later, stmt.target.name)
                for later in scoped[index + 1:]
            )
        ):
            result.append(ExprStmt(stmt.value, stmt.pos))
            continue
        result.append(stmt)
    return result


def _count_uses(node: object, counts: dict):
    """Recursively count identifier uses in an AST node."""
    if isinstance(node, NameExpr):
        counts[node.name] = counts.get(node.name, 0) + 1
    elif isinstance(node, AssignStmt):
        _count_uses(node.target, counts)
        _count_uses(node.value, counts)
    elif isinstance(node, BinaryExpr):
        _count_uses(node.left, counts)
        _count_uses(node.right, counts)
    elif isinstance(node, UnaryExpr):
        _count_uses(node.operand, counts)
    elif isinstance(node, CastExpr):
        _count_uses(node.expr, counts)
    elif isinstance(node, DotExpr):
        _count_uses(node.object, counts)
    elif isinstance(node, DotCallExpr):
        _count_uses(node.object, counts)
        for arg in node.args:
            _count_uses(arg, counts)
    elif isinstance(node, CallExpr):
        for arg in node.args:
            _count_uses(arg, counts)
    elif isinstance(node, ArrayAccessExpr):
        _count_uses(node.array, counts)
        _count_uses(node.index, counts)
    elif isinstance(node, NewArrayExpr):
        _count_uses(node.size, counts)
    elif isinstance(node, ReturnStmt):
        if node.value:
            _count_uses(node.value, counts)
    elif isinstance(node, ExprStmt):
        _count_uses(node.expr, counts)
    elif isinstance(node, IfStmt):
        _count_uses(node.condition, counts)
        for s in node.body:
            _count_uses(s, counts)
        for cond, body in node.elseif_clauses:
            _count_uses(cond, counts)
            for s in body:
                _count_uses(s, counts)
        for s in node.else_body:
            _count_uses(s, counts)
    elif isinstance(node, WhileStmt):
        _count_uses(node.condition, counts)
        for s in node.body:
            _count_uses(s, counts)


def _substitute_temps(node: object, inline_map: dict) -> object:
    """Recursively replace temp name references with their inlined expressions."""
    if isinstance(node, NameExpr):
        if node.name in inline_map:
            return inline_map[node.name]
        return node
    elif isinstance(node, AssignStmt):
        return AssignStmt(
            _substitute_temps(node.target, inline_map),
            node.op,
            _substitute_temps(node.value, inline_map),
            node.pos,
        )
    elif isinstance(node, BinaryExpr):
        return BinaryExpr(
            _substitute_temps(node.left, inline_map),
            node.op,
            _substitute_temps(node.right, inline_map),
            node.pos,
        )
    elif isinstance(node, UnaryExpr):
        return UnaryExpr(node.op, _substitute_temps(node.operand, inline_map), node.pos)
    elif isinstance(node, CastExpr):
        expr = _substitute_temps(node.expr, inline_map)
        if (
            isinstance(expr, CastExpr)
            and expr.target_type.lower() == node.target_type.lower()
        ):
            return expr
        return CastExpr(expr, node.target_type, node.pos)
    elif isinstance(node, DotExpr):
        return DotExpr(_substitute_temps(node.object, inline_map), node.member, node.pos)
    elif isinstance(node, DotCallExpr):
        return DotCallExpr(
            _substitute_temps(node.object, inline_map),
            node.method,
            [_substitute_temps(a, inline_map) for a in node.args],
            node.pos,
        )
    elif isinstance(node, CallExpr):
        return CallExpr(
            node.function,
            [_substitute_temps(a, inline_map) for a in node.args],
            node.pos,
        )
    elif isinstance(node, ArrayAccessExpr):
        return ArrayAccessExpr(
            _substitute_temps(node.array, inline_map),
            _substitute_temps(node.index, inline_map),
            node.pos,
        )
    elif isinstance(node, NewArrayExpr):
        return NewArrayExpr(
            node.element_type,
            _substitute_temps(node.size, inline_map),
            node.pos,
        )
    elif isinstance(node, ReturnStmt):
        val = _substitute_temps(node.value, inline_map) if node.value else None
        return ReturnStmt(val, node.pos)
    elif isinstance(node, ExprStmt):
        return ExprStmt(_substitute_temps(node.expr, inline_map), node.pos)
    elif isinstance(node, IfStmt):
        return IfStmt(
            _substitute_temps(node.condition, inline_map),
            [_substitute_temps(s, inline_map) for s in node.body],
            [(cond, [_substitute_temps(s, inline_map) for s in body])
             for cond, body in node.elseif_clauses],
            [_substitute_temps(s, inline_map) for s in node.else_body],
            node.pos,
        )
    elif isinstance(node, WhileStmt):
        return WhileStmt(
            _substitute_temps(node.condition, inline_map),
            [_substitute_temps(s, inline_map) for s in node.body],
            node.pos,
        )
    return node


def _cleanup_nonevar(stmts: list) -> list:
    """Convert assignments to ::nonevar/::NoneVar into expression statements.
    Also recurse into If/While bodies."""
    result = []
    for stmt in stmts:
        if isinstance(stmt, AssignStmt) and isinstance(stmt.target, NameExpr):
            if stmt.target.name.lower() == "::nonevar":
                result.append(ExprStmt(stmt.value, stmt.pos))
                continue
        if isinstance(stmt, IfStmt):
            stmt = IfStmt(
                stmt.condition,
                _cleanup_nonevar(stmt.body),
                [(c, _cleanup_nonevar(b)) for c, b in stmt.elseif_clauses],
                _cleanup_nonevar(stmt.else_body),
                stmt.pos,
            )
        elif isinstance(stmt, WhileStmt):
            stmt = WhileStmt(stmt.condition, _cleanup_nonevar(stmt.body), stmt.pos)
        result.append(stmt)
    return result


def _sanitize_internal_names(node: object) -> object:
    if isinstance(node, NameExpr):
        return NameExpr(_source_identifier_name(node.name), node.pos)
    elif isinstance(node, LocalVarStmt):
        return LocalVarStmt(
            _source_identifier_name(node.name),
            node.type,
            _sanitize_internal_names(node.value) if node.value else None,
            node.pos,
        )
    elif isinstance(node, AssignStmt):
        return AssignStmt(
            _sanitize_internal_names(node.target), node.op,
            _sanitize_internal_names(node.value), node.pos,
        )
    elif isinstance(node, BinaryExpr):
        return BinaryExpr(
            _sanitize_internal_names(node.left), node.op,
            _sanitize_internal_names(node.right), node.pos,
        )
    elif isinstance(node, UnaryExpr):
        return UnaryExpr(node.op, _sanitize_internal_names(node.operand), node.pos)
    elif isinstance(node, CastExpr):
        return CastExpr(_sanitize_internal_names(node.expr), node.target_type, node.pos)
    elif isinstance(node, DotExpr):
        return DotExpr(_sanitize_internal_names(node.object), node.member, node.pos)
    elif isinstance(node, DotCallExpr):
        return DotCallExpr(
            _sanitize_internal_names(node.object), node.method,
            [_sanitize_internal_names(a) for a in node.args], node.pos,
        )
    elif isinstance(node, CallExpr):
        return CallExpr(node.function, [_sanitize_internal_names(a) for a in node.args], node.pos)
    elif isinstance(node, ArrayAccessExpr):
        return ArrayAccessExpr(
            _sanitize_internal_names(node.array),
            _sanitize_internal_names(node.index), node.pos,
        )
    elif isinstance(node, NewArrayExpr):
        return NewArrayExpr(
            node.element_type,
            _sanitize_internal_names(node.size),
            node.pos,
        )
    elif isinstance(node, ReturnStmt):
        val = _sanitize_internal_names(node.value) if node.value else None
        return ReturnStmt(val, node.pos)
    elif isinstance(node, ExprStmt):
        return ExprStmt(_sanitize_internal_names(node.expr), node.pos)
    elif isinstance(node, IfStmt):
        return IfStmt(
            _sanitize_internal_names(node.condition),
            [_sanitize_internal_names(s) for s in node.body],
            [(c, [_sanitize_internal_names(s) for s in b]) for c, b in node.elseif_clauses],
            [_sanitize_internal_names(s) for s in node.else_body],
            node.pos,
        )
    elif isinstance(node, WhileStmt):
        return WhileStmt(
            _sanitize_internal_names(node.condition),
            [_sanitize_internal_names(s) for s in node.body], node.pos,
        )
    return node


def _rewrite_auto_vars(node: object, var_map: dict[str, str]) -> object:
    """Rewrite auto-property backing variable references (::PropName_var) to property names."""
    if isinstance(node, NameExpr):
        if node.name in var_map:
            return NameExpr(var_map[node.name], P)
        if node.name.startswith("::") and node.name.endswith("_var"):
            return NameExpr(node.name[2:-4], P)
        return node
    elif isinstance(node, AssignStmt):
        return AssignStmt(
            _rewrite_auto_vars(node.target, var_map), node.op,
            _rewrite_auto_vars(node.value, var_map), node.pos,
        )
    elif isinstance(node, BinaryExpr):
        return BinaryExpr(
            _rewrite_auto_vars(node.left, var_map), node.op,
            _rewrite_auto_vars(node.right, var_map), node.pos,
        )
    elif isinstance(node, UnaryExpr):
        return UnaryExpr(node.op, _rewrite_auto_vars(node.operand, var_map), node.pos)
    elif isinstance(node, CastExpr):
        return CastExpr(_rewrite_auto_vars(node.expr, var_map), node.target_type, node.pos)
    elif isinstance(node, DotExpr):
        return DotExpr(_rewrite_auto_vars(node.object, var_map), node.member, node.pos)
    elif isinstance(node, DotCallExpr):
        return DotCallExpr(
            _rewrite_auto_vars(node.object, var_map), node.method,
            [_rewrite_auto_vars(a, var_map) for a in node.args], node.pos,
        )
    elif isinstance(node, CallExpr):
        return CallExpr(node.function, [_rewrite_auto_vars(a, var_map) for a in node.args], node.pos)
    elif isinstance(node, ArrayAccessExpr):
        return ArrayAccessExpr(
            _rewrite_auto_vars(node.array, var_map),
            _rewrite_auto_vars(node.index, var_map), node.pos,
        )
    elif isinstance(node, NewArrayExpr):
        return NewArrayExpr(
            node.element_type,
            _rewrite_auto_vars(node.size, var_map),
            node.pos,
        )
    elif isinstance(node, ReturnStmt):
        val = _rewrite_auto_vars(node.value, var_map) if node.value else None
        return ReturnStmt(val, node.pos)
    elif isinstance(node, ExprStmt):
        return ExprStmt(_rewrite_auto_vars(node.expr, var_map), node.pos)
    elif isinstance(node, IfStmt):
        return IfStmt(
            _rewrite_auto_vars(node.condition, var_map),
            [_rewrite_auto_vars(s, var_map) for s in node.body],
            [(_rewrite_auto_vars(c, var_map), [_rewrite_auto_vars(s, var_map) for s in b])
             for c, b in node.elseif_clauses],
            [_rewrite_auto_vars(s, var_map) for s in node.else_body], node.pos,
        )
    elif isinstance(node, WhileStmt):
        return WhileStmt(
            _rewrite_auto_vars(node.condition, var_map),
            [_rewrite_auto_vars(s, var_map) for s in node.body], node.pos,
        )
    return node


# --- Full script decompilation ---

def decompile_pex_file(
    pex: PexFile,
    *,
    type_adapter: TypeAdapter | None = None,
    drop_script_const: bool = False,
    skip_internal_functions: bool = False,
    fo4_api_compat: bool = False,
) -> ScriptNode:
    """Decompile a complete PexFile into a ScriptNode AST.

    Takes the first object in the PEX file (PEX files typically contain one script).
    """
    if not pex.objects:
        return ScriptNode(name="EmptyScript", pos=P)

    obj = pex.objects[0]
    return _decompile_object(
        obj,
        pex,
        type_adapter=type_adapter,
        drop_script_const=drop_script_const,
        skip_internal_functions=skip_internal_functions,
        fo4_api_compat=fo4_api_compat,
    )


def _decompile_object(
    obj: PexObject,
    pex: PexFile,
    *,
    type_adapter: TypeAdapter | None = None,
    drop_script_const: bool = False,
    skip_internal_functions: bool = False,
    fo4_api_compat: bool = False,
) -> ScriptNode:
    """Decompile a PexObject into a ScriptNode."""
    # User flag name lookup
    flag_names = {uf.index: uf.name for uf in pex.user_flags}
    struct_names = {struct.name.lower(): struct.name for struct in obj.structs}

    def normalize_type(type_name: str) -> str:
        normalized = _normalize_pex_type_name(type_name, obj.name, struct_names)
        if type_adapter is None or _type_base_name(normalized).lower() in struct_names:
            return normalized
        return type_adapter(normalized)

    # Script-level flags
    script_flags = _decode_user_flags(obj.user_flags, flag_names)
    if drop_script_const:
        script_flags = [flag for flag in script_flags if flag.lower() != "const"]
    elif obj.is_const and not any(flag.lower() == "const" for flag in script_flags):
        script_flags.append("Const")

    # Build auto-property backing var → property name map
    auto_var_map = {}
    for p in obj.properties:
        if p.auto_var:
            auto_var_map[p.auto_var] = p.name
    variables_by_name = {var.name: var for var in obj.variables}

    member_types: dict[str, str] = {}
    for var in obj.variables:
        member_types[var.name] = normalize_type(var.type)
    for prop in obj.properties:
        prop_type = normalize_type(prop.type)
        member_types[prop.name] = prop_type
        if prop.auto_var:
            member_types[prop.auto_var] = prop_type

    # Variables (skip auto-property backing vars)
    auto_var_names = set(auto_var_map.keys())
    variables = []
    for var in obj.variables:
        if var.name in auto_var_names:
            continue
        var_flags = _decode_user_flags(var.user_flags, flag_names)
        ast_var = VariableDef(
            name=var.name,
            type=normalize_type(var.type),
            value=_pex_value_to_literal(var.data),
            flags=var_flags,
            pos=P,
        )
        variables.append(ast_var)

    # Properties
    properties = []
    for prop in obj.properties:
        prop_flags = []
        if prop.flags & 4:  # autovar
            if prop.flags & 1 and not (prop.flags & 2):  # read-only auto
                prop_flags.append("AutoReadOnly")
            else:
                prop_flags.append("Auto")
        prop_flags.extend(_decode_user_flags(prop.user_flags, flag_names))
        # The compiler moves Conditional off the property and onto its backing
        # auto-variable, so recovering it means reading the variable's flags.
        auto_var = variables_by_name.get(prop.auto_var or "")
        if auto_var is not None:
            already = {flag.lower() for flag in prop_flags}
            prop_flags.extend(
                flag
                for flag in _decode_user_flags(auto_var.user_flags, flag_names)
                if flag.lower() == "conditional" and flag.lower() not in already
            )
        getter = None
        if prop.getter is not None:
            getter = FunctionDef(
                name="Get",
                return_type=normalize_type(prop.getter.return_type),
                params=[
                    Parameter(p.name, normalize_type(p.type), pos=P)
                    for p in prop.getter.params
                ],
                is_native=prop.getter.is_native,
                is_global=prop.getter.is_global,
                docstring=prop.getter.docstring,
                body=decompile_function(
                    prop.getter,
                    auto_var_map,
                    normalize_type,
                    emit_locals=True,
                    external_types=member_types,
                    fo4_api_compat=fo4_api_compat,
                ),
                pos=P,
            )
        setter = None
        if prop.setter is not None:
            setter = FunctionDef(
                name="Set",
                return_type=normalize_type(prop.setter.return_type),
                params=[
                    Parameter(p.name, normalize_type(p.type), pos=P)
                    for p in prop.setter.params
                ],
                is_native=prop.setter.is_native,
                is_global=prop.setter.is_global,
                docstring=prop.setter.docstring,
                body=decompile_function(
                    prop.setter,
                    auto_var_map,
                    normalize_type,
                    emit_locals=True,
                    external_types=member_types,
                    fo4_api_compat=fo4_api_compat,
                ),
                pos=P,
            )

        ast_prop = PropertyDef(
            name=prop.name,
            type=normalize_type(prop.type),
            flags=prop_flags,
            docstring=prop.docstring,
            default=(
                _pex_value_to_literal(variables_by_name[prop.auto_var].data)
                if prop.auto_var in variables_by_name
                else None
            ),
            getter=getter,
            setter=setter,
            pos=P,
        )
        properties.append(ast_prop)

    structs = []
    for struct in obj.structs:
        members = []
        for member in struct.members:
            member_flags = _decode_user_flags(member.user_flags, flag_names)
            if member.is_const:
                member_flags.append("Const")
            members.append(
                StructMemberDef(
                    name=member.name,
                    type=normalize_type(member.type),
                    value=_pex_value_to_literal(member.data),
                    flags=member_flags,
                    pos=P,
                )
            )
        structs.append(StructDef(name=struct.name, members=members, pos=P))

    # Functions and events from states
    functions = []
    events = []
    named_states = []

    for state in obj.states:
        state_fns = []
        state_evts = []
        for fn in state.functions:
            # `::remote_<Sender>_<Event>` is how BOTH FO76 and FO4 encode a custom
            # event handler (`Event <Sender>.<Event>(...)`). It is compiler output,
            # not an internal helper — dropping it silently leaves the script
            # registering for an event it can no longer handle.
            remote_event = _remote_event_parts(fn)
            if (
                skip_internal_functions
                and fn.name.startswith("::")
                and remote_event is None
            ):
                continue
            # Events: return None and name starts with "On" (or matches known names)
            is_event = (
                fn.name.lower() in _EVENT_NAMES
                or (fn.return_type in ("None", "NONE", "none", "")
                    and fn.name.lower().startswith("on"))
            )

            body = decompile_function(
                fn,
                auto_var_map,
                normalize_type,
                emit_locals=True,
                external_types=member_types,
                fo4_api_compat=fo4_api_compat,
            )
            params = [Parameter(p.name, normalize_type(p.type), pos=P) for p in fn.params]
            if (
                fo4_api_compat
                and is_event
                and obj.parent.lower() == "activemagiceffect"
                and fn.name.lower() == "oneffectstart"
                and len(params) == 4
                and params[2].type.lower() == "float"
                and params[3].type.lower() == "float"
            ):
                dropped_params = params[2:]
                params = params[:2]
                used_dropped_params = [
                    param for param in dropped_params
                    if any(_node_references_name(stmt, param.name) for stmt in body)
                ]
                body = [
                    _compat_local_for_dropped_param(param)
                    for param in used_dropped_params
                ] + body
            if (
                fo4_api_compat
                and is_event
                and obj.parent.lower() == "activemagiceffect"
                and fn.name.lower() == "oneffectfinish"
                and len(params) == 5
                and all(param.type.lower() == "float" for param in params[2:])
            ):
                dropped_params = params[2:]
                params = params[:2]
                body = [
                    _compat_local_for_dropped_param(param)
                    for param in dropped_params
                    if any(_node_references_name(stmt, param.name) for stmt in body)
                ] + body
            if (
                fo4_api_compat
                and is_event
                and fn.name.lower() == "onhit"
                and len(params) == 10
                and params[8].type.lower() == "bool"
                and params[9].type.lower() == "string"
            ):
                dropped_param = params.pop(8)
                if any(_node_references_name(stmt, dropped_param.name) for stmt in body):
                    body = [_compat_local_for_dropped_param(dropped_param)] + body
            if (
                fo4_api_compat
                and is_event
                and fn.name.lower() == "onradiationdamage"
                and len(params) == 3
                and params[1].type.lower() == "float"
                and params[2].type.lower() == "bool"
            ):
                dropped_param = params.pop(1)
                if any(_node_references_name(stmt, dropped_param.name) for stmt in body):
                    body = [_compat_local_for_dropped_param(dropped_param)] + body
            if (
                fo4_api_compat
                and is_event
                and obj.parent.lower() == "topicinfo"
                and fn.name.lower() in {"onbegin", "onend"}
                and len(params) == 4
                and params[0].type.lower() == "objectreference"
                and params[1].type.lower() == "objectreference"
                and params[2].type.lower() in {"quest", "questinstance"}
                and params[3].type.lower() == "bool"
            ):
                target_param = params[1]
                quest_param = params[2]
                params = [params[0], params[3]]
                compatibility_locals = []
                if any(_node_references_name(stmt, target_param.name) for stmt in body):
                    compatibility_locals.append(LocalVarStmt(
                        target_param.name,
                        target_param.type,
                        DotCallExpr(NameExpr("Game", P), "GetPlayer", [], P),
                        P,
                    ))
                if any(_node_references_name(stmt, quest_param.name) for stmt in body):
                    compatibility_locals.append(LocalVarStmt(
                        quest_param.name,
                        quest_param.type,
                        DotCallExpr(NameExpr("Self", P), "GetOwningQuest", [], P),
                        P,
                    ))
                body = compatibility_locals + body

            if remote_event is not None:
                sender, event_name = remote_event
                state_evts.append(EventDef(
                    name=f"{sender}.{event_name}", params=params,
                    is_native=fn.is_native, docstring=fn.docstring,
                    body=body, pos=P,
                ))
            elif is_event:
                ast_node = EventDef(
                    name=fn.name, params=params, is_native=fn.is_native,
                    docstring=fn.docstring, body=body, pos=P,
                )
                state_evts.append(ast_node)
            else:
                ast_node = FunctionDef(
                    name=fn.name, return_type=normalize_type(fn.return_type), params=params,
                    is_native=fn.is_native, is_global=fn.is_global,
                    docstring=fn.docstring, body=body, pos=P,
                )
                state_fns.append(ast_node)

        if state.name == "":
            # Default/unnamed state — add to script-level
            functions.extend(state_fns)
            events.extend(state_evts)
        else:
            is_auto = state.name == obj.auto_state
            named_states.append(StateDef(
                name=state.name, is_auto=is_auto,
                functions=state_fns, events=state_evts, pos=P,
            ))

    script = ScriptNode(
        name=obj.name,
        parent=normalize_type(obj.parent) if obj.parent else None,
        flags=script_flags,
        variables=variables,
        properties=properties,
        functions=functions,
        events=events,
        states=named_states,
        structs=structs,
        pos=P,
    )
    _restore_custom_event_names(script, member_types)
    return script


_CUSTOM_EVENT_SEND = {"sendcustomevent"}
_CUSTOM_EVENT_REGISTER = {"registerforcustomevent", "unregisterforcustomevent"}


def _remote_event_parts(fn) -> tuple[str, str] | None:
    """Split `::remote_<Sender>_<Event>` using the handler's own `akSender` param
    type — the script name itself can contain underscores, so the name alone is
    ambiguous."""
    prefix = "::remote_"
    if not fn.name.startswith(prefix) or not fn.params:
        return None
    raw = fn.name[len(prefix):]
    sender = _type_base_name(fn.params[0].type)
    if sender and raw.lower().startswith(f"{sender.lower()}_"):
        return sender, raw[len(sender) + 1:]
    return None


def _strip_event_prefix(literal: str, owner: str) -> str | None:
    if not owner or not isinstance(literal, str):
        return None
    if literal.lower().startswith(f"{owner.lower()}_"):
        return literal[len(owner) + 1:]
    return None


def _restore_custom_event_names(script: ScriptNode, member_types: dict) -> None:
    """Rewrite custom-event calls from compiler-internal form back to source form.

    A .pex stores the event name fully qualified (`<sender>_<Event>`) because the
    compiler prefixes it at build time. Emitting that literal back into .psc makes
    the next compile prefix it a SECOND time, so sender and receiver end up with
    different strings and the event can never be delivered. The sender argument of
    Register/UnregisterForCustomEvent is likewise stored with an explicit cast to
    ScriptObject, which would make the prefix `scriptobject_`.
    """
    lowered_members = {str(k).lower(): v for k, v in (member_types or {}).items()}

    def sender_type_of(expr) -> str:
        if isinstance(expr, CastExpr):
            return sender_type_of(expr.expr)
        if isinstance(expr, NameExpr):
            return _type_base_name(lowered_members.get(expr.name.lower(), ""))
        return ""

    def visit(node):
        if isinstance(node, DotCallExpr):
            method = str(node.method or "").lower()
            if method in _CUSTOM_EVENT_SEND and node.args:
                lit = node.args[0]
                if isinstance(lit, LiteralExpr):
                    bare = _strip_event_prefix(lit.value, script.name)
                    if bare is not None:
                        node.args[0] = LiteralExpr(bare, lit.type, lit.pos)
            elif method in _CUSTOM_EVENT_REGISTER and len(node.args) >= 2:
                sender = sender_type_of(node.args[0])
                lit = node.args[1]
                if isinstance(lit, LiteralExpr):
                    bare = _strip_event_prefix(lit.value, sender)
                    if bare is not None:
                        node.args[1] = LiteralExpr(bare, lit.type, lit.pos)
                # The cast to ScriptObject is compiler-inserted; keeping it makes
                # the recompiled prefix `scriptobject_` instead of the real type.
                arg0 = node.args[0]
                if (
                    isinstance(arg0, CastExpr)
                    and str(arg0.target_type).lower() == "scriptobject"
                ):
                    node.args[0] = arg0.expr
        for value in vars(node).values() if hasattr(node, "__dict__") else ():
            if isinstance(value, list):
                for item in value:
                    if hasattr(item, "__dict__"):
                        visit(item)
            elif hasattr(value, "__dict__"):
                visit(value)

    for holder in (script.functions, script.events):
        for fn in holder:
            for stmt in fn.body:
                visit(stmt)
    for state in script.states:
        for fn in list(state.functions) + list(state.events):
            for stmt in fn.body:
                visit(stmt)


def _normalize_pex_type_name(
    type_name: str,
    current_script_name: str,
    struct_names: dict[str, str],
) -> str:
    suffix = ""
    base = str(type_name or "")
    while base.endswith("[]"):
        suffix += "[]"
        base = base[:-2]
    if "#" not in base:
        return f"{base}{suffix}"

    script_name, struct_name = base.split("#", 1)
    struct_name = struct_names.get(struct_name.lower(), struct_name)
    if script_name.lower() == current_script_name.lower():
        return f"{struct_name}{suffix}"
    return f"{script_name}:{struct_name}{suffix}"


def _type_base_name(type_name: str) -> str:
    base = str(type_name or "")
    while base.endswith("[]"):
        base = base[:-2]
    return base


def _decode_user_flags(flags: int, flag_names: dict[int, str]) -> list[str]:
    """Decode a user flags bitfield into flag name strings."""
    result = []
    for bit, name in flag_names.items():
        if flags & (1 << bit):
            result.append(name)
    return result


def _pex_value_to_literal(val: PexValue) -> object | None:
    """Convert a PexValue default to an AST LiteralExpr, or None if NONE."""
    if val.type == ValueType.NONE:
        return None
    return _value_to_expr(val)
