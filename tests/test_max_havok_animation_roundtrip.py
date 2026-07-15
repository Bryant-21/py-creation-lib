import os
from pathlib import Path

import pytest

from creation_lib.max.havok import (
    import_hkx_animation_document,
    export_hkx_animation_document,
)


FO4_DATA = os.environ.get("FO4_DATA", "")
SAMPLES = [
    "Meshes/Actors/Character/Animations/1HM/AttackSprinting.hkx",
    "Meshes/Actors/Character/Animations/Idle/IdleStart.hkx",
    "Meshes/Actors/Character/Animations/WalkForward.hkx",
]


@pytest.mark.skipif(not FO4_DATA, reason="FO4_DATA env var not set")
@pytest.mark.parametrize("sample", SAMPLES)
def test_animation_roundtrip_is_loadable(tmp_path: Path, sample: str) -> None:
    source = Path(FO4_DATA) / sample
    if not source.exists():
        pytest.skip(f"{source} not present")
    doc = import_hkx_animation_document(str(source))
    out = tmp_path / "roundtrip.hkx"
    result = export_hkx_animation_document(doc, str(out))
    assert out.exists()
    assert out.stat().st_size > 0
    # Re-import the round-trip output — if anything is malformed this raises.
    doc2 = import_hkx_animation_document(str(out))
    assert doc2.get("num_frames") == doc.get("num_frames")
