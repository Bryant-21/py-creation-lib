from pathlib import Path

from creation_lib.ba2 import native_runtime as ba2
from creation_lib.dds.native_runtime import write_dds_rgba
from creation_lib.material_tools.texture_upgrader import place_outputs


def _write(path: Path, data: bytes = b"x") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def _members(archive: Path) -> set[str]:
    return {name.replace("\\", "/").lower() for name in ba2.list_archive(str(archive))}


def test_outputs_follow_how_the_mod_ships(tmp_path: Path) -> None:
    mod = tmp_path / "MyMod"
    _write(mod / "MyMod.esp")
    _write(mod / "materials/loose/thing.bgsm", b"old")
    packed = tmp_path / "packed"
    _write(packed / "meshes/a.nif", b"old")
    ba2.pack_archive(str(packed), str(mod / "MyMod - Main.ba2"), "fo4")

    staging = tmp_path / "staging"
    keys = ["materials/loose/thing.bgsm", "meshes/a.nif", "meshes/b.nif", "textures/new_r.dds"]
    for key in keys[:3]:
        _write(staging / key, b"new")
    assert write_dds_rgba(str(staging / keys[3]), 4, 4, bytes(64), format="BC1_UNORM", use_gpu=False)

    out = mod / "upgraded"
    placed = place_outputs(staging, keys, mod, out)

    assert (out / "materials/loose/thing.bgsm").read_bytes() == b"new", "loose stays loose"
    assert placed == {"loose": 1, "archives": ["MyMod - Main.ba2", "MyMod - Textures.ba2"]}
    assert _members(out / "MyMod - Main.ba2") == {"meshes/a.nif", "meshes/b.nif"}
    assert ba2.extract_one(str(out / "MyMod - Main.ba2"), "meshes/a.nif") == b"new"
    assert ba2.archive_info(str(out / "MyMod - Textures.ba2"))["format"] == "fo4_dx10"
    assert _members(mod / "MyMod - Main.ba2") == {"meshes/a.nif"}, "the mod is untouched without replace"


def test_loose_only_mod_stays_loose(tmp_path: Path) -> None:
    mod = tmp_path / "Loose"
    _write(mod / "Loose.esp")
    staging = tmp_path / "staging"
    _write(staging / "textures/x_l.dds", b"new")
    assert place_outputs(staging, ["textures/x_l.dds"], mod, mod) == {"loose": 1, "archives": []}
    assert (mod / "textures/x_l.dds").read_bytes() == b"new"
