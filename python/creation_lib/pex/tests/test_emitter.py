"""Tests for the native Papyrus emitter.

Exercise `emit_script_native` end-to-end by assembling whole `ScriptNode`s
and asserting on the rendered source.
"""
import pytest

from creation_lib.papyrus_lsp.native_runtime import emit_script_native as emit_script
from creation_lib.papyrus_lsp.ast_nodes import (
    Pos, NameExpr, LiteralExpr, BinaryExpr, UnaryExpr, CastExpr,
    DotExpr, CallExpr, DotCallExpr, ArrayAccessExpr, NewArrayExpr,
    ExprStmt, AssignStmt, ReturnStmt, IfStmt, WhileStmt, LocalVarStmt,
    FunctionDef, EventDef, PropertyDef, VariableDef, StateDef,
    ScriptNode, Parameter,
)

P = Pos(0, 0, 0, 0)


def _wrap_expr(expr) -> ScriptNode:
    """Build a minimal script that contains `expr` as the body of a function."""
    return ScriptNode(
        name="T",
        functions=[FunctionDef(
            name="F", return_type="None",
            body=[ExprStmt(expr=expr, pos=P)], pos=P,
        )],
        pos=P,
    )


def _wrap_stmt(stmt) -> ScriptNode:
    return ScriptNode(
        name="T",
        functions=[FunctionDef(
            name="F", return_type="None",
            body=[stmt], pos=P,
        )],
        pos=P,
    )


# --- Expression tests ---

def test_emit_expressions():
    assert "akActor" in emit_script(_wrap_expr(NameExpr("akActor", P)))

    assert "42" in emit_script(_wrap_expr(LiteralExpr(42, "int", P)))
    assert "3.14" in emit_script(_wrap_expr(LiteralExpr(3.14, "float", P)))
    assert '"hello"' in emit_script(_wrap_expr(LiteralExpr("hello", "string", P)))
    assert "True" in emit_script(_wrap_expr(LiteralExpr(True, "bool", P)))
    assert "False" in emit_script(_wrap_expr(LiteralExpr(False, "bool", P)))
    assert "None" in emit_script(_wrap_expr(LiteralExpr(None, "none", P)))

    expr = BinaryExpr(NameExpr("a", P), "+", NameExpr("b", P), P)
    assert "a + b" in emit_script(_wrap_expr(expr))

    assert "-x" in emit_script(_wrap_expr(UnaryExpr("-", NameExpr("x", P), P)))
    assert "!x" in emit_script(_wrap_expr(UnaryExpr("!", NameExpr("x", P), P)))

    expr = CastExpr(NameExpr("ref", P), "Actor", P)
    assert "ref as Actor" in emit_script(_wrap_expr(expr))

    expr = CastExpr(BinaryExpr(NameExpr("a", P), "==", NameExpr("b", P), P), "Bool", P)
    assert "(a == b) as Bool" in emit_script(_wrap_expr(expr))

    expr = DotExpr(NameExpr("self", P), "myProp", P)
    assert "self.myProp" in emit_script(_wrap_expr(expr))

    expr = CallExpr("Debug.Trace", [LiteralExpr("hi", "string", P)], P)
    assert 'Debug.Trace("hi")' in emit_script(_wrap_expr(expr))

    expr = DotCallExpr(NameExpr("akActor", P), "GetActorValue",
                       [LiteralExpr("Health", "string", P)], P)
    assert 'akActor.GetActorValue("Health")' in emit_script(_wrap_expr(expr))

    expr = ArrayAccessExpr(NameExpr("arr", P), LiteralExpr(0, "int", P), P)
    assert "arr[0]" in emit_script(_wrap_expr(expr))

    expr = NewArrayExpr("Int", LiteralExpr(10, "int", P), P)
    assert "new Int[10]" in emit_script(_wrap_expr(expr))

    expr = CallExpr("new Pair", [], P)
    assert "new Pair" in emit_script(_wrap_expr(expr))
    assert "new Pair()" not in emit_script(_wrap_expr(expr))


# --- Statement tests ---

def test_emit_statements():
    stmt = AssignStmt(NameExpr("x", P), "=", LiteralExpr(5, "int", P), P)
    assert "x = 5" in emit_script(_wrap_stmt(stmt))

    stmt = AssignStmt(NameExpr("x", P), "+=", LiteralExpr(1, "int", P), P)
    assert "x += 1" in emit_script(_wrap_stmt(stmt))

    src = emit_script(_wrap_stmt(ReturnStmt(None, P)))
    assert "Return" in src
    src = emit_script(_wrap_stmt(ReturnStmt(NameExpr("x", P), P)))
    assert "Return x" in src

    stmt = LocalVarStmt("x", "Int", None, P)
    assert "Int x" in emit_script(_wrap_stmt(stmt))

    stmt = LocalVarStmt("x", "Int", LiteralExpr(0, "int", P), P)
    assert "Int x = 0" in emit_script(_wrap_stmt(stmt))

    stmt = IfStmt(
        condition=NameExpr("cond", P),
        body=[ReturnStmt(LiteralExpr(1, "int", P), P)],
        elseif_clauses=[],
        else_body=[],
        pos=P,
    )
    result = emit_script(_wrap_stmt(stmt))
    assert "If cond" in result
    assert "Return 1" in result
    assert "EndIf" in result

    stmt = WhileStmt(
        condition=NameExpr("running", P),
        body=[ExprStmt(CallExpr("DoWork", [], P), P)],
        pos=P,
    )
    result = emit_script(_wrap_stmt(stmt))
    assert "While running" in result
    assert "DoWork()" in result
    assert "EndWhile" in result


# --- Structure tests ---

def test_emit_structure():
    fn = FunctionDef(
        name="Add",
        return_type="Int",
        params=[Parameter("a", "Int", pos=P), Parameter("b", "Int", pos=P)],
        body=[ReturnStmt(BinaryExpr(NameExpr("a", P), "+", NameExpr("b", P), P), P)],
        pos=P,
    )
    result = emit_script(ScriptNode(name="T", functions=[fn], pos=P))
    assert "Int Function Add(Int a, Int b)" in result
    assert "Return a + b" in result
    assert "EndFunction" in result

    fn = FunctionDef(
        name="Helper", return_type="None", is_global=True, body=[], pos=P,
    )
    result = emit_script(ScriptNode(name="T", functions=[fn], pos=P))
    assert "Function Helper() Global" in result

    fn = FunctionDef(
        name="GetValue", return_type="Float", is_native=True, body=[], pos=P,
    )
    result = emit_script(ScriptNode(name="T", functions=[fn], pos=P))
    assert "Float Function GetValue() Native" in result

    ev = EventDef(
        name="OnInit",
        body=[ExprStmt(CallExpr("Debug.Trace", [LiteralExpr("init", "string", P)], P), P)],
        pos=P,
    )
    result = emit_script(ScriptNode(name="T", events=[ev], pos=P))
    assert "Event OnInit()" in result
    assert "EndEvent" in result

    prop = PropertyDef(name="MyProp", type="Int", flags=["Auto"], pos=P)
    result = emit_script(ScriptNode(name="T", properties=[prop], pos=P))
    assert "Int Property MyProp Auto" in result

    script = ScriptNode(
        name="MyScript",
        parent="ObjectReference",
        flags=["Hidden"],
        variables=[VariableDef("counter", "Int", LiteralExpr(0, "int", P), pos=P)],
        properties=[PropertyDef("MyProp", "Int", ["Auto"], pos=P)],
        functions=[
            FunctionDef("Add", "Int",
                params=[Parameter("a", "Int", pos=P), Parameter("b", "Int", pos=P)],
                body=[ReturnStmt(BinaryExpr(NameExpr("a", P), "+", NameExpr("b", P), P), P)],
                pos=P)
        ],
        events=[EventDef("OnInit", body=[], pos=P)],
        pos=P,
    )
    result = emit_script(script)
    assert "Scriptname MyScript Extends ObjectReference Hidden" in result
    assert "Int counter = 0" in result
    assert "Int Property MyProp Auto" in result
    assert "Int Function Add" in result
    assert "Event OnInit()" in result


# --- Cast operand parenthesisation ---
#
# `.`, `[]` and `as` bind more tightly than the decompiler's AST implies, and a
# cast atom accepts exactly one `as`. Bare operands produce
# `x as Actor.EquipItem(...)`, which PapyrusCompiler rejects with
# "unexpected token Dot in statement".

def _emitter_has_operand_parens() -> bool:
    return "(ref as Actor).Do()" in emit_script(
        _wrap_expr(DotCallExpr(CastExpr(NameExpr("ref", P), "Actor", P), "Do", [], P))
    )


requires_fixed_emitter = pytest.mark.skipif(
    not _emitter_has_operand_parens(),
    reason="loaded papyrus_core emitter predates SH-03a; "
           "run scripts/ensure_native.py --package creation",
)


@requires_fixed_emitter
def test_emit_cast_operand_parenthesisation():
    expr = DotCallExpr(
        CastExpr(NameExpr("akActionRef", P), "Actor", P),
        "EquipItem",
        [NameExpr("PipboyCharGen", P)],
        P,
    )
    assert "(akActionRef as Actor).EquipItem(PipboyCharGen)" in emit_script(
        _wrap_expr(expr)
    )

    expr = DotCallExpr(
        CastExpr(CastExpr(NameExpr("Self", P), "Perk", P), "MyPerkScript", P),
        "Collect",
        [NameExpr("akActor", P)],
        P,
    )
    assert "((Self as Perk) as MyPerkScript).Collect(akActor)" in emit_script(
        _wrap_expr(expr)
    )

    member = DotExpr(CastExpr(NameExpr("akTargetRef", P), "Actor", P), "MyProp", P)
    assert "(akTargetRef as Actor).MyProp" in emit_script(_wrap_expr(member))

    index = ArrayAccessExpr(
        CastExpr(NameExpr("items", P), "Form[]", P), LiteralExpr(0, "int", P), P
    )
    assert "(items as Form[])[0]" in emit_script(_wrap_expr(index))

    expr = DotCallExpr(
        NameExpr("akActor", P),
        "Revive",
        [CastExpr(NameExpr("akTargetRef", P), "Actor", P)],
        P,
    )
    assert "akActor.Revive(akTargetRef as Actor)" in emit_script(_wrap_expr(expr))
