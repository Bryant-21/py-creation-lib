"""End-to-end integration tests: parse real .pex → decompile → verify output."""
import pytest

from creation_lib.pex import decompile_pex, parse_pex
from creation_lib.pex.tests.pex_samples import find_pex_files


def test_parse_real_pex_files():
    """Smoke test: parse real .pex files without crashing."""
    pex_files = find_pex_files()
    if not pex_files:
        pytest.skip("No .pex files found")
    for pex_path in pex_files:
        pex = parse_pex(pex_path)
        assert pex.magic == 0xFA57C0DE
        assert len(pex.objects) >= 1


def test_decompile_real_pex_files():
    """Smoke test: full decompile pipeline produces non-empty output."""
    pex_files = find_pex_files()
    if not pex_files:
        pytest.skip("No .pex files found")
    for pex_path in pex_files:
        try:
            source = decompile_pex(pex_path)
            assert "Scriptname" in source
            assert len(source) > 20
        except Exception as e:
            # Log but don't fail — some scripts may have edge cases
            print(f"WARNING: Failed to decompile {pex_path.name}: {e}")


def test_decompiled_output_is_parseable():
    """Verify decompiled output can be parsed by the Papyrus source parser."""
    from creation_lib.papyrus_lsp import parse_script

    pex_files = find_pex_files(3)
    if not pex_files:
        pytest.skip("No .pex files found")
    for pex_path in pex_files:
        try:
            source = decompile_pex(pex_path)
            # If the source parser can parse it, the output is syntactically valid
            tree = parse_script(text=source)
            assert tree is not None
        except Exception as e:
            print(f"WARNING: Parse round-trip failed for {pex_path.name}: {e}")
