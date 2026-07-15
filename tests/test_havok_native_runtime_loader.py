from types import SimpleNamespace

import numpy as np
import pytest


@pytest.fixture(autouse=True)
def restore_native_runtime_state():
    from creation_lib.havok import native_runtime

    old_module = native_runtime._NATIVE_MODULE
    old_attempted = native_runtime._NATIVE_IMPORT_ATTEMPTED
    yield
    native_runtime._NATIVE_MODULE = old_module
    native_runtime._NATIVE_IMPORT_ATTEMPTED = old_attempted


def test_load_native_module_falls_back_to_umbrella_submodule(monkeypatch):
    from creation_lib.havok import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    umbrella_module = SimpleNamespace(
        havok_native=SimpleNamespace(hkx_roundtrip_bytes=lambda data: data),
    )
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name in {"havok_native", "havok_native.havok_native"}:
            raise ImportError(name)
        if name == "creation_lib._native":
            return umbrella_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is umbrella_module.havok_native
    assert calls == ["havok_native", "creation_lib._native"]


def test_load_native_module_falls_back_to_umbrella_extension(monkeypatch):
    from creation_lib.havok import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    extension_module = SimpleNamespace(
        havok_native=SimpleNamespace(hkx_roundtrip_bytes=lambda data: data),
    )
    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        if name in {"havok_native", "havok_native.havok_native"}:
            raise ImportError(name)
        if name == "creation_lib._native":
            return extension_module
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    module = native_runtime.load_native_module()

    assert module is extension_module.havok_native
    assert calls == [
        "havok_native",
        "creation_lib._native",
    ]


def test_load_native_module_tries_umbrella_fallback_once(monkeypatch):
    from creation_lib.havok import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = False

    calls: list[str] = []

    def _fake_import(name: str):
        calls.append(name)
        raise ImportError(name)

    monkeypatch.setattr(native_runtime, "import_module", _fake_import)

    assert native_runtime.load_native_module() is None
    assert calls == [
        "havok_native",
        "creation_lib._native",
    ]


def test_raw_helpers_forward_to_native_functions(monkeypatch):
    from creation_lib.havok import native_runtime

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_detect_format=lambda data: "packfile" if data.startswith(b"\x57") else "tagfile",
        hkx_roundtrip_bytes=lambda data: b"rt:" + data,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.hkx_detect_format_raw(b"\x57abc") == "packfile"
    assert native_runtime.hkx_roundtrip_bytes_raw(b"abc") == b"rt:abc"


def test_collision_blob_builders_normalize_numpy_vertices(monkeypatch):
    from creation_lib.havok import native_runtime

    calls = []

    def polytope(vertices, *_args):
        calls.append(vertices)
        return b"ok"

    def compressed(vertices, triangles, *_args):
        calls.append((vertices, triangles))
        return b"ok"

    def starfield(vertices, *_args):
        calls.append(vertices)
        return b"ok"

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_roundtrip_bytes=lambda data: data,
        fo4_polytope_collision_blob=polytope,
        fo4_compressed_mesh_collision_blob=compressed,
        starfield_convex_collision_blob=starfield,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    vertices = np.array([[1, 2, 3], [4, 5, 6]], dtype=np.float32)
    triangles = np.array([[0, 1, 2]], dtype=np.int32)

    assert native_runtime.fo4_polytope_collision_blob_raw(vertices, 0.5, 0.4, 1, 0.0) == b"ok"
    assert native_runtime.fo4_compressed_mesh_collision_blob_raw(vertices, triangles) == b"ok"
    assert native_runtime.starfield_convex_collision_blob_raw(vertices) == b"ok"
    assert calls == [
        [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        (
            [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
            [[0, 1, 2]],
        ),
        [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
    ]


def test_hkx_detect_format_raw_unwraps_native_tuple(monkeypatch):
    from creation_lib.havok import native_runtime

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_detect_format=lambda data: ("packfile", "hk_2014.1.0-r1"),
        hkx_roundtrip_bytes=lambda data: data,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.hkx_detect_format_raw(b"abc") == "packfile"


def test_walk_meshes_dir_native_passes_source(monkeypatch):
    from creation_lib.havok import native_runtime

    calls = []

    def walk_meshes_dir(root_path, source):
        calls.append((root_path, source))
        return '[{"rel_path": "a.hkx"}]'

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_roundtrip_bytes=lambda data: data,
        walk_meshes_dir=walk_meshes_dir,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.walk_meshes_dir_native("Meshes", "fo4") == [{"rel_path": "a.hkx"}]
    assert calls == [("Meshes", "fo4")]


def test_build_manifests_native_json_encodes_payloads(monkeypatch):
    from creation_lib.havok import native_runtime

    calls = []

    def build_manifests(entries_json, character_data_json, source):
        calls.append((entries_json, character_data_json, source))
        return "[]"

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_roundtrip_bytes=lambda data: data,
        build_manifests=build_manifests,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert native_runtime.build_manifests_native([{"rel_path": "a.hkx"}], {}, "fo4") == []
    assert calls == [('[{"rel_path": "a.hkx"}]', "{}", "fo4")]


def test_write_animation_xml_native_matches_native_signature(monkeypatch):
    from creation_lib.havok import native_runtime

    calls = []

    def havok_write_animation_xml(clip_json, skeleton_bone_names):
        calls.append((clip_json, skeleton_bone_names))
        return "<hkpackfile />"

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_roundtrip_bytes=lambda data: data,
        havok_write_animation_xml=havok_write_animation_xml,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    assert (
        native_runtime.write_animation_xml_native({"name": "clip"}, ["Root"])
        == "<hkpackfile />"
    )
    assert calls == [('{"name": "clip"}', ["Root"])]


def test_decompress_spline_native_expands_param_dict(monkeypatch):
    from creation_lib.havok import native_runtime

    calls = []

    def havok_decompress_spline(*args):
        calls.append(args)
        return "[]"

    native_runtime._NATIVE_MODULE = SimpleNamespace(
        hkx_roundtrip_bytes=lambda data: data,
        havok_decompress_spline=havok_decompress_spline,
    )
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    params = {
        "num_transform_tracks": 1,
        "num_float_tracks": 2,
        "num_frames": 3,
        "max_frames_per_block": 4,
        "num_blocks": 5,
        "block_offsets": [6],
        "float_block_offsets": [7],
        "mask_and_quant_size": 8,
        "block_duration": 9.0,
        "block_inverse_duration": 10.0,
        "frame_duration": 11.0,
    }

    assert native_runtime.decompress_spline_native(b"blob", params) == []
    assert calls == [
        (
            b"blob",
            1,
            2,
            3,
            4,
            5,
            [6],
            [7],
            8,
            9.0,
            10.0,
            11.0,
        )
    ]


def test_missing_native_module_raises_runtime_error(monkeypatch):
    from creation_lib.havok import native_runtime

    native_runtime._NATIVE_MODULE = None
    native_runtime._NATIVE_IMPORT_ATTEMPTED = True

    with pytest.raises(RuntimeError, match="havok_native is not available"):
        native_runtime.hkx_roundtrip_bytes_raw(b"abc")
