"""Canary test: ensure converted Meltdown BGSMs have cubemap envmap coverage."""
from __future__ import annotations

import glob
from pathlib import Path

from creation_lib.material_tools.bgsm_bin import read_bgsm


def test_meltdown_envmap_coverage():
    repo_root = Path(__file__).resolve().parent.parent.parent
    pattern = str(repo_root / "mods/B21_Converted_meltdown_Batch/data/Materials/**/*.bgsm")
    paths = glob.glob(pattern, recursive=True)
    total = 0
    with_env = 0
    for p in paths:
        total += 1
        with open(p, "rb") as f:
            b = read_bgsm(f)
        env = (b.EnvmapTexture or "").replace(chr(0), "").strip()
        if env:
            with_env += 1
    assert total > 0, f"no BGSMs found at {pattern}"
    coverage = with_env / total
    assert coverage >= 0.85, (
        f"cubemap coverage {with_env}/{total} = {coverage:.1%}; target >=85%"
    )
