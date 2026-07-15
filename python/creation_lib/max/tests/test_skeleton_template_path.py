from pathlib import Path


def test_fo4_skeleton_template_is_bundled():
    from creation_lib.max.havok import _fo4_skeleton_nif_template_path

    path = _fo4_skeleton_nif_template_path()
    assert path.is_file(), f"bundled skeleton.nif missing: {path}"
    assert "creation_lib" in str(path)
    assert "resource" not in path.parts  # never the repo-root resource/ dir


def test_bml_fuz_decode_is_bundled():
    from creation_lib.paths import get_resource_dir

    assert (get_resource_dir() / "BmlFuzDecode.exe").is_file()
