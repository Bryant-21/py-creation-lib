"""Byte-exact roundtrip tests for FO4 behavior-graph packfiles.

Behavior graphs (`hkbBehaviorGraph` + derived classes) exercise what animation
packfiles don't: `hkbBindable`-derived classes with `SERIALIZE_IGNORED` array
members (`cachedBindables`, `uniqueIdPool`, etc.). Vanilla FO4 packs those arrays
with `capacity = 0x80000000` (the "owns memory" flag at the high byte) so the
runtime allocator treats them as heap-owned empty arrays. The writer must match,
or CK crashes reconciling the bindable table on load.

Anchors:
- `DeathclawRootBehavior.hkx` (51920 bytes): contains `hkbFootIkControlsModifier`.
- `BehemothRootBehavior.hkx` (57056 bytes): a second `hkbFootIkControlsModifier`
  file, so coverage doesn't hinge on one fixture.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.hkxpack import load_hkx, write_hkx


BEHAVIOR_FILES = [
    "extracted/fo4/Meshes/Actors/Deathclaw/Behaviors/DeathclawRootBehavior.hkx",
    "extracted/fo4/Meshes/Actors/SuperMutantBehemoth/Behaviors/BehemothRootBehavior.hkx",
]


@pytest.mark.parametrize("rel_path", BEHAVIOR_FILES)
def test_behavior_roundtrip_byte_exact(rel_path: str):
    """Write-then-compare must be byte-exact against the vanilla file."""
    # Resolve relative to the repo root (three levels up from this test file).
    repo_root = Path(__file__).resolve().parents[3]
    src = repo_root / rel_path
    if not src.exists():
        pytest.skip(f"missing fixture: {src}")

    hkx, registry = load_hkx(str(src))
    out = write_hkx(hkx, registry)
    original = src.read_bytes()

    assert len(out) == len(original), (
        f"size mismatch: original={len(original)}, out={len(out)}, "
        f"diff={len(out) - len(original)}"
    )

    if out != original:
        # Report the first divergence for quicker debugging.
        for i, (a, b) in enumerate(zip(original, out)):
            if a != b:
                ctx_o = original[max(0, i - 8):i + 16].hex()
                ctx_n = out[max(0, i - 8):i + 16].hex()
                pytest.fail(
                    f"first byte diff at 0x{i:x}: orig=0x{a:02x} out=0x{b:02x}\n"
                    f"  orig ctx: {ctx_o}\n"
                    f"  out ctx:  {ctx_n}"
                )
