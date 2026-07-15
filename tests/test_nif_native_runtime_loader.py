from types import SimpleNamespace

import pytest


@pytest.fixture(autouse=True)
def restore_native_runtime_state():
    from creation_lib.nif import native_runtime

    old_module = native_runtime._NATIVE_MODULE
    old_attempted = native_runtime._NATIVE_IMPORT_ATTEMPTED
    yield
    native_runtime._NATIVE_MODULE = old_module
    native_runtime._NATIVE_IMPORT_ATTEMPTED = old_attempted


def test_load_native_module_falls_back_to_umbrella_submodule(monkeypatch):
    from creation_lib.nif import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    umbrella_module = SimpleNamespace(
        nif_core_native=SimpleNamespace(load_nif=lambda path: {"path": path}),
    )
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name in {"nif_core_native", "nif_core_native.nif_core_native"}:
            raise ImportError(name)
        if name == "creation_lib._native":
            return umbrella_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is umbrella_module.nif_core_native
    assert calls == ["nif_core_native", "creation_lib._native"]


def test_load_native_module_falls_back_to_umbrella_extension(monkeypatch):
    from creation_lib.nif import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    extension_module = SimpleNamespace(
        nif_core_native=SimpleNamespace(load_nif=lambda path: {"path": path}),
    )
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name in {"nif_core_native", "nif_core_native.nif_core_native"}:
            raise ImportError(name)
        if name == "creation_lib._native":
            return extension_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is extension_module.nif_core_native
    assert calls == [
        "nif_core_native",
        "creation_lib._native",
    ]


def test_load_nif_raw_uses_function_payload(monkeypatch):
    from creation_lib.nif import native_runtime

    payload = {"header": {}, "blocks": []}
    native_runtime._NATIVE_MODULE = SimpleNamespace(load_nif=lambda path: payload | {"path": path})
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.load_nif_raw("example.nif") == {
        "header": {},
        "blocks": [],
        "path": "example.nif",
    }


def test_bytes_helpers_use_function_payloads(monkeypatch):
    from creation_lib.nif import native_runtime

    payload = {"header": {}, "blocks": []}
    native_runtime._NATIVE_MODULE = SimpleNamespace(
        nif_from_bytes=lambda data: payload | {"size": len(data)},
        nif_to_bytes=lambda raw: b"NIF" + bytes([len(raw["blocks"])]),
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.nif_from_bytes_raw(b"abc") == {
        "header": {},
        "blocks": [],
        "size": 3,
    }
    assert native_runtime.nif_to_bytes_raw(payload) == b"NIF\x00"


def test_new_nif_raw_uses_native_function(monkeypatch):
    from creation_lib.nif import native_runtime

    payload = {"header": {"version": (20, 2, 0, 7)}, "blocks": []}
    native_runtime._NATIVE_MODULE = SimpleNamespace(new_nif=lambda game: payload | {"game": game})
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.new_nif_raw("fo4") == {
        "header": {"version": (20, 2, 0, 7)},
        "blocks": [],
        "game": "fo4",
    }


def test_nif_file_from_native_dict_payload():
    from creation_lib.nif.nif_file import NifFile

    nif = NifFile._from_native(
        {
            "header": {
                "version": (20, 2, 0, 7),
                "user_version": 12,
                "bs_version": 130,
                "num_blocks": 1,
                "footer_roots": [0],
            },
            "blocks": [
                {
                    "block_id": 0,
                    "type_name": "BSFadeNode",
                    "fields": {"Name": "Root", "Controller": -1},
                    "remainder": b"extra",
                }
            ],
        }
    )

    assert nif.header.version == (20, 2, 0, 7)
    assert nif.header.bs_version == 130
    assert nif._footer_roots == [0]
    assert nif.blocks[0].type_name == "BSFadeNode"
    assert nif.blocks[0].get_field("Name") == "Root"
    assert nif.blocks[0]._remainder == b"extra"


def test_nif_file_load_does_not_fall_back_to_python_reader(monkeypatch):
    from creation_lib.nif import native_runtime
    from creation_lib.nif.nif_file import NifFile

    def _missing(path: str):
        raise RuntimeError("nif_core_native is not available")

    monkeypatch.setattr(native_runtime, "load_nif_raw", _missing)

    with pytest.raises(RuntimeError, match="nif_core_native is not available"):
        NifFile.load("missing-native.nif")


def test_nif_file_save_uses_native_writer(monkeypatch, tmp_path):
    from creation_lib.nif import native_runtime
    from creation_lib.nif.nif_file import NifBlock, NifFile

    nif = NifFile()
    nif.header.block_type_names = ["BSFadeNode"]
    nif.header.block_type_index = [0]
    nif.header.block_sizes = [0]
    block = NifBlock(block_id=0, type_name="BSFadeNode")
    block.set_field("Name", "")
    block.set_field("Controller", -1)
    block.set_field("Flags", 14)
    block.set_field("Translation", [1.0, 2.0, 3.0])
    block.set_field("Rotation", [[1, 0, 0], [0, 1, 0], [0, 0, 1]])
    block.set_field("Scale", 1.0)
    block.set_field("Num Children", 0)
    block.set_field("Children", [])
    nif.blocks.append(block)

    calls = []

    def _save(payload, path):
        calls.append((payload, path))

    monkeypatch.setattr(native_runtime, "save_nif_raw", _save)
    out = tmp_path / "out.nif"

    nif.save(str(out))

    assert calls[0][1] == str(out)
    fields = calls[0][0]["blocks"][0]["fields"]
    assert fields["Name"] == ""
    assert fields["Translation"] == {"x": 1.0, "y": 2.0, "z": 3.0}
    assert fields["Rotation"] == {
        "m11": 1,
        "m12": 0,
        "m13": 0,
        "m21": 0,
        "m22": 1,
        "m23": 0,
        "m31": 0,
        "m32": 0,
        "m33": 1,
    }


def test_nif_file_save_round_trips_new_bstrishape(tmp_path):
    from creation_lib.nif.nif_file import NifFile

    nif = NifFile.new("fo4")
    root = nif.get_block(0)
    assert root is not None

    shape = nif.add_block("BSTriShape")
    shape.set_field("Name", "CoreTriangle")
    shape.set_field("Num Vertices", 3)
    shape.set_field("Vertex Desc", 193_514_046_685_700)
    shape.set_field(
        "Vertex Data",
        [
            _vertex_data((0.0, 0.0, 0.0), (0.0, 0.0)),
            _vertex_data((1.0, 0.0, 0.0), (1.0, 0.0)),
            _vertex_data((0.0, 1.0, 0.0), (0.0, 1.0)),
        ],
    )
    shape.set_field("Num Triangles", 1)
    shape.set_field("Triangles", [{"v1": 0, "v2": 1, "v3": 2}])
    shader = nif.add_block("BSLightingShaderProperty")
    texture_set = nif.add_block("BSShaderTextureSet")
    texture_set.set_field("Num Textures", 8)
    texture_set.set_field(
        "Textures",
        [
            "textures/core_triangle_d.dds",
            "textures/core_triangle_n.dds",
            "",
            "",
            "",
            "",
            "",
            "textures/core_triangle_s.dds",
        ],
    )
    shader.set_field("Texture Set", texture_set.block_id)
    shape.set_field("Shader Property", shader.block_id)
    root.set_field("Children", [shape.block_id])
    root.set_field("Num Children", 1)

    out = tmp_path / "core_triangle.nif"
    nif.save(str(out))

    exported = NifFile.load(str(out))
    exported_root = exported.get_block(0)
    assert exported_root is not None
    assert exported_root.get_field("Children") == [shape.block_id]
    exported_shape = exported.get_block(shape.block_id)
    assert exported_shape is not None
    assert exported_shape.get_field("Name") == "CoreTriangle"
    assert exported_shape.get_field("Num Vertices") == 3
    assert len(exported_shape.get_field("Vertex Data")) == 3
    assert exported_shape.get_field("Num Triangles") == 1
    assert exported_shape.get_field("Data Size") == 54
    assert exported_shape.get_field("Triangles")[0] == {"v1": 0, "v2": 1, "v3": 2}
    exported_shader = exported.get_block(exported_shape.get_field("Shader Property"))
    assert exported_shader is not None
    exported_texture_set = exported.get_block(exported_shader.get_field("Texture Set"))
    assert exported_texture_set is not None
    assert exported_texture_set.get_field("Textures")[0] == "textures/core_triangle_d.dds"
    assert exported_texture_set.get_field("Textures")[7] == "textures/core_triangle_s.dds"


def _vertex_data(vertex, uv):
    return {
        "Vertex": {"x": vertex[0], "y": vertex[1], "z": vertex[2]},
        "Unused W": 0,
        "UV": {"u": uv[0], "v": uv[1]},
        "Normal": {"x": 0.0, "y": 0.0, "z": 1.0},
        "Bitangent Y": 0.0,
    }
