"""Headers synthesized from `.pex` must carry the type facts `.psc` carried.

Fallout 4 ships only compiled `.pex`; the `.psc` arrive with the Creation Kit.
These cover the two facts `.pex` cannot express on its own — default parameter
values and which declarations are events — plus the shape of the emitted text.
"""
from __future__ import annotations

from types import SimpleNamespace

from creation_lib.pex.headers import emit_header

_MANIFEST = {
    "defaults": {"widget": {"additem": {"1": "1", "2": "false"}}},
    "events": {"widget": ["oninit"]},
}


def _fn(name, return_type="None", params=(), is_native=True, is_global=False):
    return SimpleNamespace(
        name=name,
        return_type=return_type,
        params=[SimpleNamespace(name=n, type=t) for n, t in params],
        is_native=is_native,
        is_global=is_global,
    )


def _obj(name="Widget", parent="Form", functions=(), properties=(), structs=(), states=None):
    return SimpleNamespace(
        name=name,
        parent=parent,
        structs=list(structs),
        properties=list(properties),
        states=list(states) if states is not None
        else [SimpleNamespace(name="", functions=list(functions))],
    )


def test_scriptname_parent_and_root_extends():
    assert emit_header(_obj(), _MANIFEST).startswith("Scriptname Widget Extends Form")

    header = emit_header(_obj(name="ScriptObject", parent=""), _MANIFEST)
    assert header.startswith("Scriptname ScriptObject\n")
    assert "Extends" not in header


def test_function_and_event_signatures():
    """Without manifest defaults every optional parameter becomes required;
    events must stay Events since the compiler validates them against
    inherited signatures."""
    fn = _fn("AddItem", params=(("akItem", "Form"), ("aiCount", "Int"), ("abSilent", "Bool")))
    header = emit_header(_obj(functions=[fn]), _MANIFEST)
    assert "Function AddItem(Form akItem, Int aiCount = 1, Bool abSilent = false)" in header

    header = emit_header(
        _obj(functions=[_fn("OnInit", is_native=False)]), _MANIFEST
    )
    assert "Event OnInit()" in header
    assert "EndEvent" in header
    assert "Function OnInit" not in header

    header = emit_header(_obj(functions=[_fn("GetFormID", return_type="Int")]), _MANIFEST)
    assert "Int Function GetFormID() Native" in header

    header = emit_header(_obj(functions=[_fn("Reset")]), _MANIFEST)
    assert "Function Reset() Native" in header
    assert "None Function" not in header

    # Globals compile to CALLSTATIC, so the keyword is load-bearing.
    fn = _fn("RandomFloat", return_type="Float", is_global=True)
    header = emit_header(_obj(name="Utility", functions=[fn]), _MANIFEST)
    assert "Float Function RandomFloat() Global Native" in header


def test_properties_and_structs_carry_types():
    prop = SimpleNamespace(name="Health", type="Float", flags=7, auto_var="::Health_var")
    header = emit_header(_obj(properties=[prop]), _MANIFEST)
    assert "Float Property Health Auto" in header

    struct = SimpleNamespace(
        name="StatAchievement",
        members=[
            SimpleNamespace(name="StatName", type="String"),
            SimpleNamespace(name="Threshold", type="Int"),
        ],
    )
    header = emit_header(_obj(structs=[struct]), _MANIFEST)
    assert "Struct StatAchievement" in header
    assert "    String StatName" in header
    assert "    Int Threshold" in header
    assert "EndStruct" in header


def test_a_name_declared_in_several_states_is_emitted_once():
    """States repeat a signature; the type universe needs it exactly once."""
    header = emit_header(
        _obj(states=[
            SimpleNamespace(name="", functions=[_fn("Toggle")]),
            SimpleNamespace(name="Busy", functions=[_fn("Toggle")]),
        ]),
        _MANIFEST,
    )

    assert header.count("Function Toggle(") == 1


def test_emitting_without_a_manifest_entry_still_produces_a_valid_header():
    header = emit_header(_obj(name="Unknown", functions=[_fn("Go")]), _MANIFEST)

    assert "Scriptname Unknown Extends Form" in header
    assert "Function Go() Native" in header


def test_struct_type_spelling_is_source_legal():
    """`.pex` writes `script#struct`; `#` is a parse error in source.

    One bad declaration takes down the whole header, so the resolver caches a
    None AST for that script and every call into it resolves to void. A single
    `quest#queststage` in Quest.psc caused 25 "cannot assign None to Int"
    failures scattered across unrelated scripts.
    """
    fn = _fn("GetQuestStageDone", return_type="Bool",
             params=(("questStageToCheck", "quest#queststage"),), is_global=True)
    header = emit_header(_obj(name="Quest", functions=[fn]), _MANIFEST)

    assert "#" not in header
    # Inside its own declaring script the struct is referenced bare.
    assert "queststage questStageToCheck" in header

    # A struct type from another script is colon-qualified.
    fn = _fn("Take", params=(("data", "other#payload"),))
    header = emit_header(_obj(name="Widget", functions=[fn]), _MANIFEST)
    assert "other:payload data" in header


def test_compiler_mangled_names_are_omitted():
    """`::remote_*` is not source-legal and crashes the parser outright.

    Emitting one drove a 137438953472-byte allocation that aborted the process.
    """
    fn = _fn("::remote_REParentScript_RECheckForCleanup",
             params=(("akSender", "reparentscript"), ("akArgs", "Var[]")))
    prop = SimpleNamespace(name="::hidden_var", type="Int", flags=7, auto_var="")
    header = emit_header(
        _obj(name="REScript", functions=[fn, _fn("Reset")], properties=[prop]),
        _MANIFEST,
    )

    assert "::" not in header
    assert "Function Reset() Native" in header
