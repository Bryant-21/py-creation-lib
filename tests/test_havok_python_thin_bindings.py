from __future__ import annotations

from pathlib import Path

from creation_lib.animation.models import AnimationClip, AnimationEvent, AnimationKeyframe, BoneChannel


def test_parse_skeleton_delegates_to_native(monkeypatch, tmp_path):
    from creation_lib.havok import native_runtime
    from creation_lib.havok.parsers.skeleton import parse_skeleton

    calls: list[str] = []

    def parse_skeleton_xml_native(xml: str) -> dict:
        calls.append(xml)
        return {
            "name": "NativeSkeleton",
            "bone_count": 1,
            "bone_names": ["Root"],
            "parent_indices": [-1],
            "reference_pose": [{"t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0], "s": [1.0, 1.0, 1.0]}],
            "lock_translation": [False],
            "float_count": 0,
            "float_slots": [],
            "reference_floats": [],
            "partition_names": ["Body"],
        }

    monkeypatch.setattr(native_runtime, "parse_skeleton_xml_native", parse_skeleton_xml_native)
    path = tmp_path / "skeleton.xml"
    path.write_text("<hkpackfile />", encoding="utf-8")

    result = parse_skeleton(path)

    assert calls == ["<hkpackfile />"]
    assert result.name == "NativeSkeleton"
    assert result.bone_names == ["Root"]
    assert result.partition_names == ["Body"]


def test_discover_havok_files_delegates_to_native(monkeypatch, tmp_path):
    from creation_lib.havok import native_runtime
    from creation_lib.havok.discovery import discover_havok_files

    calls: list[tuple[str, str]] = []

    def walk_meshes_dir_native(root_path: str, source: str) -> list[dict]:
        calls.append((root_path, source))
        return [
            {
                "rel_path": "UniqueBehaviors/Test/Behaviors/Behavior.xml",
                "role": "behavior",
                "category": "Weapon",
                "file_type": "xml",
                "is_xml": True,
            }
        ]

    monkeypatch.setattr(native_runtime, "walk_meshes_dir_native", walk_meshes_dir_native)

    entries = discover_havok_files(tmp_path, source="fo4")

    assert calls == [(str(tmp_path), "fo4")]
    assert entries[0].abs_path == tmp_path / "UniqueBehaviors" / "Test" / "Behaviors" / "Behavior.xml"
    assert entries[0].role == "behavior"


def test_extract_clip_delegates_to_native(monkeypatch, tmp_path):
    from creation_lib.havok import native_runtime
    from creation_lib.havok.animation_reader import extract_clip

    calls: list[tuple[str, str | None]] = []

    def extract_clip_native(animation_xml: str, skeleton_xml: str | None = None) -> dict:
        calls.append((animation_xml, skeleton_xml))
        return {
            "name": "NativeClip",
            "duration": 1.0,
            "channels": [
                {
                    "bone_name": "Root",
                    "rotations": [{"time": 0.0, "value": [0.0, 0.0, 0.0, 1.0]}],
                    "translations": [{"time": 0.0, "value": [1.0, 2.0, 3.0]}],
                    "scales": [{"time": 0.0, "value": [1.0, 1.0, 1.0]}],
                }
            ],
            "events": [{"time": 0.5, "text": "Hit"}],
            "native_fps": 60.0,
            "source_format": "hkx",
        }

    monkeypatch.setattr(native_runtime, "extract_clip_native", extract_clip_native)
    path = tmp_path / "anim.xml"
    path.write_text("<hkpackfile />", encoding="utf-8")

    clip = extract_clip(path)

    assert calls == [("<hkpackfile />", None)]
    assert clip is not None
    assert clip.duration == 1.0
    assert clip.native_fps == 60.0
    assert clip.channels[0].bone_name == "Root"
    assert clip.events[0].text == "Hit"


def test_write_animation_xml_delegates_to_native(monkeypatch, tmp_path):
    from creation_lib.havok import native_runtime
    from creation_lib.havok.animation_writer import write_animation_xml

    calls: list[tuple[dict, list[str]]] = []

    def write_animation_xml_native(clip_dict: dict, skeleton_bone_names: list[str]) -> str:
        calls.append((clip_dict, skeleton_bone_names))
        return "<hkpackfile><native /></hkpackfile>"

    monkeypatch.setattr(native_runtime, "write_animation_xml_native", write_animation_xml_native)
    clip = AnimationClip(
        name="Clip",
        duration=1.0,
        channels=(
            BoneChannel(
                bone_name="Root",
                rotations=(AnimationKeyframe(0.0, (0.0, 0.0, 0.0, 1.0)),),
            ),
        ),
        events=(AnimationEvent(0.25, "Hit"),),
    )
    output = tmp_path / "out.xml"

    write_animation_xml(clip, ["Root"], output)

    assert output.read_text(encoding="utf-8") == "<hkpackfile><native /></hkpackfile>"
    assert calls[0][0]["name"] == "Clip"
    assert calls[0][0]["channels"][0]["bone_name"] == "Root"
    assert calls[0][1] == ["Root"]


def test_decompress_spline_delegates_to_native(monkeypatch):
    from creation_lib.havok import native_runtime
    from creation_lib.havok.spline_decompress import decompress_spline

    calls: list[tuple[bytes, dict]] = []

    def decompress_spline_native(blob_bytes: bytes, params: dict) -> list:
        calls.append((blob_bytes, params))
        return [
            [
                {
                    "translation": [1.0, 2.0, 3.0],
                    "rotation": [0.0, 0.0, 0.0, 1.0],
                    "scale": [1.0, 1.0, 1.0],
                }
            ]
        ]

    monkeypatch.setattr(native_runtime, "decompress_spline_native", decompress_spline_native)

    frames = decompress_spline(
        b"blob",
        1,
        0,
        1,
        1,
        1,
        [0],
        [],
        4,
        1.0,
        1.0,
        1.0,
    )

    assert calls[0][0] == b"blob"
    assert calls[0][1]["num_transform_tracks"] == 1
    assert frames[0][0].translation == (1.0, 2.0, 3.0)
