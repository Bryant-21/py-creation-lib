import json
import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def test_native_convert_bytes_and_file_preserve_fo4_noop(tmp_path):
    import creation_lib._native.havok_native as havok
    src = ROOT / "resource" / "skeleton.hkx"
    dst = tmp_path / "nested" / "out.hkx"
    original = src.read_bytes()

    assert bytes(havok.havok_convert_bytes(original, "fo4")) == original

    havok.havok_convert_file(str(src), str(dst), "fo4")

    assert dst.read_bytes() == original


def test_native_convert_batch_reports_mixed_results(tmp_path):
    import creation_lib._native.havok_native as havok
    src_root = tmp_path / "src"
    dst_root = tmp_path / "dst"
    (src_root / "a").mkdir(parents=True)
    (src_root / "b").mkdir(parents=True)
    shutil.copyfile(ROOT / "resource" / "skeleton.hkx", src_root / "a" / "skeleton.hkx")
    shutil.copyfile(
        ROOT / "py_creation_lib/python/creation_lib" / "conversion" / "tests" / "fixtures" / "creatures" / "deathclaw" / "expected" / "character.hkx",
        src_root / "b" / "character.hkx",
    )
    shutil.copyfile(
        ROOT / "py_creation_lib/python/creation_lib" / "hkxpack" / "tests" / "fixtures" / "fo76_snallygastercharacter.hkx",
        src_root / "b" / "fo76.hkx",
    )

    result = json.loads(havok.havok_convert_batch(str(src_root), str(dst_root), "fo4", True))

    # Phase 6: FO76 → FO4 conversion is now wired through native; all three
    # files succeed (skeleton.hkx is FO4 noop, character.hkx is FO4 noop,
    # fo76_snallygastercharacter.hkx is converted via the FO76 migration path).
    assert result["converted"] == 3
    assert result["skipped"] == 0
    assert result["errors"] == []
    assert (dst_root / "b" / "fo76.hkx").exists()
    assert (dst_root / "a" / "skeleton.hkx").read_bytes() == (ROOT / "resource" / "skeleton.hkx").read_bytes()
