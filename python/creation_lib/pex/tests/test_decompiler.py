"""Tests for PEX bytecode → AST decompilation."""
from creation_lib.pex.types import (
    PexFile, PexObject, PexState, PexFunction, PexInstruction,
    PexValue, PexVariable, PexProperty, PexParam, PexLocal, ValueType,
    PexStruct, PexStructMember, PexUserFlag,
)
from creation_lib.pex.opcodes import PexOpcode
from creation_lib.pex.decompiler import decompile_function, decompile_pex_file
from creation_lib.pex.native_runtime import compile_psc, parse_pex_bytes_native
from creation_lib.papyrus_lsp.native_runtime import emit_script_native
from creation_lib.papyrus_lsp.ast_nodes import (
    AssignStmt, BinaryExpr, NameExpr, ReturnStmt, ExprStmt,
    DotCallExpr, DotExpr, CastExpr, LiteralExpr, CallExpr,
    ArrayAccessExpr, UnaryExpr, IfStmt, WhileStmt,
    LocalVarStmt, ScriptNode, FunctionDef as AstFunctionDef,
)


def _make_id(name: str) -> PexValue:
    return PexValue.identifier(name)


def _make_int(val: int) -> PexValue:
    return PexValue.integer(val)


def _make_fn(instructions, params=None, locals_=None, return_type="None"):
    return PexFunction(
        name="TestFunc",
        return_type=return_type,
        docstring="",
        is_native=False,
        is_global=False,
        params=params or [],
        locals=locals_ or [],
        instructions=instructions,
    )


# --- Expression reconstruction tests ---

def test_decompile_assign():
    """ASSIGN x, 5 → x = 5"""
    instrs = [
        PexInstruction(PexOpcode.ASSIGN, [_make_id("x"), _make_int(5)]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    assert len(stmts) >= 1
    assert isinstance(stmts[0], AssignStmt)
    assert stmts[0].target.name == "x"
    assert stmts[0].value.value == 5


def test_decompile_iadd():
    """IADD result, a, b → result = a + b"""
    instrs = [
        PexInstruction(PexOpcode.IADD, [_make_id("result"), _make_id("a"), _make_id("b")]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    assert isinstance(stmts[0], AssignStmt)
    assert isinstance(stmts[0].value, BinaryExpr)
    assert stmts[0].value.op == "+"


def test_decompile_callmethod():
    """CALLMETHOD GetValue, akActor, result, 1, "Health" → result = akActor.GetValue("Health")"""
    instrs = [
        PexInstruction(PexOpcode.CALLMETHOD, [
            _make_id("GetValue"),   # method name
            _make_id("akActor"),    # object
            _make_id("result"),     # dest
            _make_int(1),           # arg count
            PexValue.string("Health"),  # arg
        ]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    assert isinstance(stmts[0], AssignStmt) or isinstance(stmts[0], ExprStmt)


def test_decompile_propget():
    """PROPGET MyProp, self, dest → dest = self.MyProp"""
    instrs = [
        PexInstruction(PexOpcode.PROPGET, [_make_id("MyProp"), _make_id("self"), _make_id("dest")]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    assert isinstance(stmts[0], AssignStmt)
    assert isinstance(stmts[0].value, DotExpr)


def test_decompile_cast():
    """CAST dest, src → dest = src as DestType"""
    instrs = [
        PexInstruction(PexOpcode.CAST, [_make_id("dest"), _make_id("src")]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    fn = _make_fn(instrs, locals_=[PexLocal("dest", "Actor"), PexLocal("src", "ObjectReference")])
    stmts = decompile_function(fn)
    assert isinstance(stmts[0], AssignStmt)
    assert isinstance(stmts[0].value, CastExpr)


def test_decompile_return_value():
    """RETURN x → Return x"""
    instrs = [PexInstruction(PexOpcode.RETURN, [_make_id("x")])]
    stmts = decompile_function(_make_fn(instrs, return_type="Int"))
    assert isinstance(stmts[-1], ReturnStmt)
    assert stmts[-1].value.name == "x"


def test_decompile_temp_inlining():
    """Temp variables with single use get inlined into expressions.
    ::temp0 = a + b; result = ::temp0 * c → result = (a + b) * c
    """
    instrs = [
        PexInstruction(PexOpcode.IADD, [_make_id("::temp0"), _make_id("a"), _make_id("b")]),
        PexInstruction(PexOpcode.IMUL, [_make_id("result"), _make_id("::temp0"), _make_id("c")]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    fn = _make_fn(instrs, locals_=[PexLocal("::temp0", "Int")])
    stmts = decompile_function(fn)
    # Should have 1 statement: result = (a + b) * c
    assigns = [s for s in stmts if isinstance(s, AssignStmt)]
    assert len(assigns) == 1
    assert assigns[0].target.name == "result"
    assert isinstance(assigns[0].value, BinaryExpr)
    assert assigns[0].value.op == "*"


def test_decompile_temp_reassignment_keeps_legal_target():
    instrs = [
        PexInstruction(PexOpcode.ASSIGN, [_make_id("::temp0"), _make_int(1)]),
        PexInstruction(PexOpcode.ASSIGN, [_make_id("::temp0"), _make_int(2)]),
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    assigns = [s for s in stmts if isinstance(s, AssignStmt)]
    assert [stmt.target.name for stmt in assigns] == ["temp0", "temp0"]


# --- Control flow tests ---

def test_decompile_if():
    """If pattern: JMPF cond → else_target; body; JMP → end; else_body"""
    instrs = [
        # 0: JMPF cond, +3 (jump to instruction 3 if false)
        PexInstruction(PexOpcode.JMPF, [_make_id("cond"), _make_int(3)]),
        # 1: body — assign x = 1
        PexInstruction(PexOpcode.ASSIGN, [_make_id("x"), _make_int(1)]),
        # 2: JMP +2 (jump to instruction 4, past else)
        PexInstruction(PexOpcode.JMP, [_make_int(2)]),
        # 3: else — assign x = 2
        PexInstruction(PexOpcode.ASSIGN, [_make_id("x"), _make_int(2)]),
        # 4: return
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    # Should produce an IfStmt
    if_stmts = [s for s in stmts if isinstance(s, IfStmt)]
    assert len(if_stmts) == 1
    assert len(if_stmts[0].body) >= 1
    assert len(if_stmts[0].else_body) >= 1


def test_decompile_if_no_else():
    """If without else: JMPF cond → end; body; end"""
    instrs = [
        # 0: JMPF cond, +2 (jump to instruction 2 if false)
        PexInstruction(PexOpcode.JMPF, [_make_id("cond"), _make_int(2)]),
        # 1: body
        PexInstruction(PexOpcode.ASSIGN, [_make_id("x"), _make_int(1)]),
        # 2: return
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    stmts = decompile_function(_make_fn(instrs))
    if_stmts = [s for s in stmts if isinstance(s, IfStmt)]
    assert len(if_stmts) == 1
    assert len(if_stmts[0].body) >= 1
    assert len(if_stmts[0].else_body) == 0


def test_decompile_while():
    """While pattern: condition check; JMPF → end; body; JMP → condition"""
    instrs = [
        # 0: CMP_GT ::temp, counter, 0
        PexInstruction(PexOpcode.CMP_GT, [_make_id("::temp0"), _make_id("counter"), _make_int(0)]),
        # 1: JMPF ::temp, +3 (jump to 4 if false — exit loop)
        PexInstruction(PexOpcode.JMPF, [_make_id("::temp0"), _make_int(3)]),
        # 2: ISUB counter, counter, 1
        PexInstruction(PexOpcode.ISUB, [_make_id("counter"), _make_id("counter"), _make_int(1)]),
        # 3: JMP -3 (jump back to 0 — loop)
        PexInstruction(PexOpcode.JMP, [_make_int(-3)]),
        # 4: return
        PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
    ]
    fn = _make_fn(instrs, locals_=[PexLocal("::temp0", "Bool")])
    stmts = decompile_function(fn)
    while_stmts = [s for s in stmts if isinstance(s, WhileStmt)]
    assert len(while_stmts) == 1


# --- Full script decompilation tests ---

def test_decompile_full_script():
    """Decompile a PexFile with one object, one property, one function."""
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                auto_state="",
                variables=[PexVariable("counter", "Int", 0, PexValue.integer(0))],
                properties=[PexProperty("MyProp", "Int", flags=4, auto_var="::MyProp_var")],
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="OnInit", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            params=[], locals=[],
                            instructions=[
                                PexInstruction(PexOpcode.ASSIGN, [_make_id("counter"), _make_int(1)]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )
    script = decompile_pex_file(pex)
    assert isinstance(script, ScriptNode)
    assert script.name == "MyScript"
    assert script.parent == "ObjectReference"
    assert len(script.variables) == 1
    assert len(script.properties) == 1
    assert "Auto" in script.properties[0].flags
    # OnInit should be classified as an event
    assert len(script.events) >= 1


def test_decompile_preserves_auto_property_backing_variable_default():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                variables=[
                    PexVariable("::Delay_var", "Float", 0, PexValue.floating(12.0)),
                ],
                properties=[
                    PexProperty("Delay", "Float", flags=4, auto_var="::Delay_var"),
                ],
            ),
        ],
    )

    source = emit_script_native(decompile_pex_file(pex))

    assert "Float Property Delay = 12.0 Auto" in source


def test_decompile_struct_types_are_valid_papyrus_source():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                structs=[
                    PexStruct(
                        "Pair",
                        members=[PexStructMember("First", "Int")],
                    ),
                ],
                variables=[PexVariable("Pairs", "myscript#pair[]")],
                properties=[PexProperty("PairProp", "myscript#pair[]", flags=4)],
                states=[],
            ),
        ],
    )

    script = decompile_pex_file(pex)
    source = emit_script_native(script)

    assert script.structs[0].name == "Pair"
    assert script.structs[0].members[0].type == "Int"
    assert script.variables[0].type == "Pair[]"
    assert script.properties[0].type == "Pair[]"
    assert "Struct Pair" in source
    assert "Int First" in source
    assert "Pair[] Pairs" in source
    assert "Pair[] Property PairProp Auto" in source
    assert "#" not in source


def test_decompile_type_adapter_applies_to_non_struct_types():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                variables=[PexVariable("BlastRegion", "region")],
                states=[],
            ),
        ],
    )

    script = decompile_pex_file(
        pex,
        type_adapter=lambda type_name: "Form" if type_name.lower() == "region" else type_name,
    )

    assert script.variables[0].type == "Form"


def test_decompile_type_adapter_applies_to_script_parent():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyQuestScript",
                parent="QuestInstance",
                states=[],
            ),
        ],
    )

    script = decompile_pex_file(
        pex,
        type_adapter=lambda type_name: (
            "Quest" if type_name.lower() == "questinstance" else type_name
        ),
    )

    assert script.parent == "Quest"


def test_decompile_preserves_script_level_conditional_flag():
    pex = PexFile(
        magic=0xFA57C0DE,
        major_version=3,
        minor_version=9,
        game_id=2,
        compilation_time=0,
        source_filename="ConditionalQuest.psc",
        username="",
        machine_name="",
        string_table=[],
        debug_info=None,
        user_flags=[PexUserFlag(name="conditional", index=1)],
        objects=[
            PexObject(
                name="ConditionalQuest",
                parent="Quest",
                user_flags=1 << 1,
                variables=[
                    PexVariable(
                        name="rewardReady",
                        type="Bool",
                        user_flags=1 << 1,
                        data=PexValue.boolean(False),
                    )
                ],
                states=[PexState("", functions=[])],
            )
        ],
    )

    source = emit_script_native(decompile_pex_file(pex))

    assert "Scriptname ConditionalQuest Extends Quest conditional" in source
    assert "Bool rewardReady = False conditional" in source

    compiled = compile_psc(source)
    assert compiled.ok, compiled.diagnostics
    round_tripped = parse_pex_bytes_native(compiled.pex_bytes)
    conditional_index = next(
        flag.index for flag in round_tripped.user_flags if flag.name.lower() == "conditional"
    )
    assert round_tripped.objects[0].user_flags & (1 << conditional_index)


def test_decompile_struct_ops_emit_legal_source():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                structs=[PexStruct("Pair", [PexStructMember("First", "Int")])],
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="GetFirst", return_type="Int", docstring="",
                            is_native=False, is_global=False,
                            locals=[
                                PexLocal("item", "myscript#pair"),
                                PexLocal("::temp0", "Int"),
                            ],
                            instructions=[
                                PexInstruction(PexOpcode.STRUCT_CREATE, [_make_id("item")]),
                                PexInstruction(PexOpcode.STRUCT_GET, [
                                    _make_id("::temp0"),
                                    _make_id("item"),
                                    _make_id("First"),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [_make_id("::temp0")]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    script = decompile_pex_file(pex)
    source = emit_script_native(script)

    assert not any(isinstance(stmt, LocalVarStmt) and stmt.name == "temp0"
                   for stmt in script.functions[0].body)
    assert "::temp" not in source
    assert "Pair item" in source
    assert "Int temp0" not in source
    assert "item = new Pair" in source
    assert "new Pair()" not in source
    assert "Return item.First" in source


def test_decompile_array_create_uses_element_type():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="MakeArray", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            locals=[PexLocal("items", "ObjectReference[]")],
                            instructions=[
                                PexInstruction(PexOpcode.ARRAY_CREATE, [
                                    _make_id("items"),
                                    _make_int(0),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    source = emit_script_native(decompile_pex_file(pex))

    assert "ObjectReference[] items" in source
    assert "items = new ObjectReference[0]" in source
    assert "new ObjectReference[][0]" not in source


def test_decompile_nested_same_type_casts_are_collapsed():
    instrs = [
        PexInstruction(PexOpcode.CMP_EQ, [_make_id("::temp0"), _make_id("a"), _make_id("b")]),
        PexInstruction(PexOpcode.CAST, [_make_id("::temp1"), _make_id("::temp0")]),
        PexInstruction(PexOpcode.CAST, [_make_id("::temp2"), _make_id("::temp1")]),
        PexInstruction(PexOpcode.RETURN, [_make_id("::temp2")]),
    ]
    fn = _make_fn(
        instrs,
        locals_=[
            PexLocal("::temp0", "Bool"),
            PexLocal("::temp1", "Bool"),
            PexLocal("::temp2", "Bool"),
        ],
        return_type="Bool",
    )

    stmts = decompile_function(fn)
    script = ScriptNode(
        name="MyScript",
        functions=[AstFunctionDef("Check", "Bool", body=stmts)],
    )
    source = emit_script_native(script)

    assert "as Bool as Bool" not in source
    assert "Return (a == b) as Bool" in source


def test_decompile_property_getter_body():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                properties=[
                    PexProperty(
                        "ConstValue",
                        "Int",
                        flags=1,
                        getter=PexFunction(
                            name="get_ConstValue", return_type="Int", docstring="",
                            is_native=False, is_global=False,
                            instructions=[
                                PexInstruction(PexOpcode.RETURN, [_make_int(4)]),
                            ],
                        ),
                    ),
                ],
                states=[],
            ),
        ],
    )

    source = emit_script_native(decompile_pex_file(pex))

    assert "Int Property ConstValue" in source
    assert "Int Function Get()" in source
    assert "Return 4" in source
    assert "EndProperty" in source


def test_decompile_can_drop_const_and_internal_functions_for_porting():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                is_const=True,
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="::remote_Test", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            instructions=[PexInstruction(PexOpcode.RETURN, [PexValue.none()])],
                        ),
                        PexFunction(
                            name="KeepMe", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            instructions=[PexInstruction(PexOpcode.RETURN, [PexValue.none()])],
                        ),
                    ]),
                ],
            ),
        ],
    )

    script = decompile_pex_file(
        pex,
        drop_script_const=True,
        skip_internal_functions=True,
    )

    assert "Const" not in script.flags
    assert [fn.name for fn in script.functions] == ["KeepMe"]


def test_decompile_fo4_compat_drops_sound_play_node_arg_and_rewrites_inherited_auto_var():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="PlayIt", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            locals=[PexLocal("::temp0", "Int")],
                            instructions=[
                                PexInstruction(PexOpcode.CALLMETHOD, [
                                    _make_id("Play"),
                                    _make_id("::MusicLoop_var"),
                                    _make_id("::temp0"),
                                    _make_int(2),
                                    _make_id("Self"),
                                    PexValue.string(""),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert "MusicLoop.Play(Self)" in source
    assert 'MusicLoop.Play(Self, "")' not in source
    assert "Int temp0" not in source
    assert "::MusicLoop_var" not in source


def test_decompile_fo4_compat_trims_weapon_fire_bool_arg():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="FireIt", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            locals=[PexLocal("myGun", "weapon")],
                            instructions=[
                                PexInstruction(PexOpcode.CALLMETHOD, [
                                    _make_id("Fire"),
                                    _make_id("myGun"),
                                    PexValue.none(),
                                    _make_int(3),
                                    _make_id("Self"),
                                    PexValue.none(),
                                    PexValue.boolean(True),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert "myGun.Fire(Self, None)" in source
    assert "myGun.Fire(Self, None, True)" not in source


def test_decompile_fo4_compat_rewrites_game_get_local_player():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="GetThePlayer", return_type="Actor", docstring="",
                            is_native=False, is_global=False,
                            locals=[PexLocal("::temp0", "Actor")],
                            instructions=[
                                PexInstruction(PexOpcode.CALLSTATIC, [
                                    _make_id("Game"),
                                    _make_id("GetLocalPlayer"),
                                    _make_id("::temp0"),
                                    _make_int(0),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [_make_id("::temp0")]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    default_source = emit_script_native(decompile_pex_file(pex))
    compat_source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert "Game.GetLocalPlayer()" in default_source
    assert "Game.GetPlayer()" in compat_source
    assert "GetLocalPlayer" not in compat_source


def test_decompile_fo4_compat_trims_debug_trace_category_arg():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="TraceIt", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            instructions=[
                                PexInstruction(PexOpcode.CALLSTATIC, [
                                    _make_id("Debug"),
                                    _make_id("Trace"),
                                    PexValue.none(),
                                    _make_int(3),
                                    PexValue.string("hello"),
                                    _make_int(1),
                                    PexValue.string("Creatures"),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    default_source = emit_script_native(decompile_pex_file(pex))
    compat_source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert 'Debug.Trace("hello", 1, "Creatures")' in default_source
    assert 'Debug.Trace("hello", 1)' in compat_source
    assert '"Creatures"' not in compat_source


def test_decompile_fo4_compat_drops_actor_set_equipped_weapon_attacks_enabled():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ObjectReference",
                variables=[PexVariable("selfRef", "actor")],
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="ToggleIt", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            instructions=[
                                PexInstruction(PexOpcode.CALLMETHOD, [
                                    _make_id("SetEquippedWeaponAttacksEnabled"),
                                    _make_id("selfRef"),
                                    PexValue.none(),
                                    _make_int(2),
                                    _make_int(2),
                                    PexValue.boolean(False),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    default_source = emit_script_native(decompile_pex_file(pex))
    compat_source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert "selfRef.SetEquippedWeaponAttacksEnabled(2, False)" in default_source
    assert "SetEquippedWeaponAttacksEnabled" not in compat_source


def test_decompile_fo4_compat_drops_damage_dealt_event_calls():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyScript",
                parent="ActiveMagicEffect",
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="StopListening", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            instructions=[
                                PexInstruction(PexOpcode.CALLMETHOD, [
                                    _make_id("UnregisterForAllDamageDealtEvents"),
                                    _make_id("Self"),
                                    PexValue.none(),
                                    _make_int(1),
                                    _make_id("selfActorRef"),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    default_source = emit_script_native(decompile_pex_file(pex))
    compat_source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert "UnregisterForAllDamageDealtEvents" in default_source
    assert "UnregisterForAllDamageDealtEvents" not in compat_source


def test_decompile_fo4_compat_trims_active_magic_effect_on_effect_start_params():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="MyEffect",
                parent="ActiveMagicEffect",
                variables=[PexVariable("MagnitudeSeen", "Float")],
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="OnEffectStart", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            params=[
                                PexParam("akTarget", "actor"),
                                PexParam("akCaster", "actor"),
                                PexParam("afMagnitude", "Float"),
                                PexParam("afDuration", "Float"),
                            ],
                            instructions=[
                                PexInstruction(PexOpcode.ASSIGN, [
                                    _make_id("MagnitudeSeen"),
                                    _make_id("afMagnitude"),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    default_source = emit_script_native(decompile_pex_file(pex))
    compat_source = emit_script_native(decompile_pex_file(pex, fo4_api_compat=True))

    assert "Event OnEffectStart(actor akTarget, actor akCaster, Float afMagnitude, Float afDuration)" in default_source
    assert "Event OnEffectStart(actor akTarget, actor akCaster)" in compat_source
    assert "Float afMagnitude" in compat_source
    assert "MagnitudeSeen = afMagnitude" in compat_source
    assert "afDuration" not in compat_source


def test_decompile_fo4_compat_rewrites_topic_info_event_params():
    pex = PexFile(
        magic=0xFA57C0DE, major_version=3, minor_version=9, game_id=2,
        compilation_time=0, source_filename="test.psc",
        username="u", machine_name="m",
        string_table=[],
        debug_info=None,
        user_flags=[],
        objects=[
            PexObject(
                name="DefaultTopicInfoTest",
                parent="TopicInfo",
                variables=[
                    PexVariable("SeenTarget", "ObjectReference"),
                    PexVariable("SeenQuest", "QuestInstance"),
                ],
                states=[
                    PexState("", functions=[
                        PexFunction(
                            name="OnBegin", return_type="None", docstring="",
                            is_native=False, is_global=False,
                            params=[
                                PexParam("akSpeakerRef", "ObjectReference"),
                                PexParam("akTargetRef", "ObjectReference"),
                                PexParam("akQuestInstance", "QuestInstance"),
                                PexParam("abHasBeenSaid", "Bool"),
                            ],
                            instructions=[
                                PexInstruction(PexOpcode.ASSIGN, [
                                    _make_id("SeenTarget"),
                                    _make_id("akTargetRef"),
                                ]),
                                PexInstruction(PexOpcode.ASSIGN, [
                                    _make_id("SeenQuest"),
                                    _make_id("akQuestInstance"),
                                ]),
                                PexInstruction(PexOpcode.RETURN, [PexValue.none()]),
                            ],
                        ),
                    ]),
                ],
            ),
        ],
    )

    compat_source = emit_script_native(decompile_pex_file(
        pex,
        type_adapter=lambda name: "Quest" if name.lower() == "questinstance" else name,
        fo4_api_compat=True,
    ))

    assert "Event OnBegin(ObjectReference akSpeakerRef, Bool abHasBeenSaid)" in compat_source
    assert "ObjectReference akTargetRef = Game.GetPlayer()" in compat_source
    assert "Quest akQuestInstance = Self.GetOwningQuest()" in compat_source
    assert "SeenTarget = akTargetRef" in compat_source
    assert "SeenQuest = akQuestInstance" in compat_source
