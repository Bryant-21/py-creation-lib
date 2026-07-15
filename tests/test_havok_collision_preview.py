from __future__ import annotations


def test_blob_preview_delegates_to_native(monkeypatch) -> None:
    from creation_lib.havok import native_runtime
    from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob

    calls: list[tuple[bytes, float, int | None]] = []

    def collision_preview_native(
        blob: bytes,
        havok_scale: float = 1.0,
        body_id: int | None = None,
    ) -> dict:
        calls.append((blob, havok_scale, body_id))
        return {
            "meshes": [
                {
                    "shape_type": "sphere",
                    "mesh": {
                        "vertices": [{"x": 0.0, "y": 0.0, "z": 0.0}],
                        "triangles": [],
                    },
                }
            ]
        }

    monkeypatch.setattr(native_runtime, "collision_preview_native", collision_preview_native)

    previews = extract_preview_meshes_from_blob(b"blob", havok_scale=70.0, body_id=2)

    assert calls == [(b"blob", 70.0, 2)]
    assert previews[0]["shape_type"] == "sphere"


def test_blob_preview_returns_empty_list_when_native_has_no_meshes(monkeypatch) -> None:
    from creation_lib.havok import native_runtime
    from creation_lib.havok.collision_preview import extract_preview_meshes_from_blob

    monkeypatch.setattr(
        native_runtime,
        "collision_preview_native",
        lambda _blob, havok_scale=1.0, body_id=None: {"meshes": []},
    )

    assert extract_preview_meshes_from_blob(b"blob", havok_scale=1.0) == []
