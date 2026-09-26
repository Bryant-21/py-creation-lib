import pytest

from creation_lib.max.havok import (
    import_hkx_animation_document,
    export_hkx_animation_document,
)


def _synthetic_document() -> dict:
    return {
        "format_version": 1,
        "kind": "havok_clip_json",
        "game": "fo4",
        "experimental": False,
        "clip": {
            "name": "TestClip",
            "duration": 1.0,
            "native_fps": 30.0,
            "is_additive": False,
            "original_skeleton_name": "",
            "track_to_bone_indices": [],
            "events": [],
            "channels": [
                {
                    "bone_name": "Root",
                    "priority": 0,
                    "rotations": [{"time": 0.0, "value": [0.0, 0.0, 0.0, 1.0]}],
                    "translations": [{"time": 0.0, "value": [0.0, 0.0, 0.0]}],
                    "scales": [{"time": 0.0, "value": [1.0, 1.0, 1.0]}],
                }
            ],
        },
        "skeleton": {"bone_order": ["Root"]},
    }


def test_animation_roundtrip_is_loadable(tmp_path):
    doc = _synthetic_document()
    out = tmp_path / "roundtrip.hkx"

    export_hkx_animation_document(doc, str(out))
    assert out.exists()
    assert out.stat().st_size > 0

    # Re-import the round-trip output — if anything is malformed this raises.
    doc2 = import_hkx_animation_document(str(out))
    assert doc2["clip"]["duration"] == pytest.approx(doc["clip"]["duration"])
    assert len(doc2["clip"]["channels"]) == 1
