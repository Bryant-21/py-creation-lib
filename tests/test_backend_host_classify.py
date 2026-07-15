import json
import os
import subprocess
import sys
from pathlib import Path

from creation_lib.max.morphs import build_morph_document
from creation_lib.animation.kf_reader import read_kf
from creation_lib.animation.kf_writer import write_kf
from creation_lib.animation.models import AnimationClip, AnimationKeyframe, BoneChannel


REPO_ROOT = Path(__file__).resolve().parents[2]
KF_TEST_HOOK_ENV = "MODKIT_MAX_NIF_BACKEND_HOST_TEST_KF_STUB"


def test_classify_skeleton_subcommand(tmp_path):
    input_path = tmp_path / "in.json"
    output_path = tmp_path / "out.json"
    input_path.write_text(
        json.dumps(
            {
                "bone_names": [
                    "Root", "Spine1",
                    "L_UpperArm", "L_ForeArm1", "L_Hand",
                    "R_UpperArm", "R_ForeArm1", "R_Hand",
                ],
                "parent_indices": [-1, 0, 1, 2, 3, 1, 5, 6],
            }
        ),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "classify-skeleton",
            "--input-json", str(input_path),
            "--output-json", str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr

    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert "chains" in payload
    assert "mirror_pairs" in payload
    assert "categories" in payload
    tips = {c["tip"] for c in payload["chains"]}
    assert "L_Hand" in tips
    assert "R_Hand" in tips


def test_export_morph_subcommand_writes_loadable_tri(tmp_path):
    input_path = tmp_path / "scene.json"
    output_path = tmp_path / "head.tri"
    input_path.write_text(
        json.dumps(
            {
                "kind": "max_morph_document",
                "game": "fo4",
                "sidecar_kind": "tri",
                "base_mesh": {
                    "vertices": [
                        {"x": 0.0, "y": 0.0, "z": 0.0},
                        {"x": 1.0, "y": 0.0, "z": 0.0},
                        {"x": 0.0, "y": 1.0, "z": 0.0},
                    ],
                    "faces": [[0, 1, 2]],
                    "uvs": [
                        {"u": 0.0, "v": 0.0},
                        {"u": 1.0, "v": 0.0},
                        {"u": 0.0, "v": 1.0},
                    ],
                    "uv_faces": [[0, 1, 2]],
                },
                "targets": [
                    {
                        "name": "Smile",
                        "shape_name": "Head",
                        "offsets": [
                            {"vertex": 1, "x": 0.125, "y": 0.0, "z": 0.0}
                        ],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "export-morph",
            "--input-json", str(input_path),
            "--output", str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    assert output_path.exists()

    loaded = build_morph_document(output_path, known_shape_names=["Head"])
    assert loaded["sidecar_kind"] == "tri"
    assert loaded["targets"][0]["name"] == "Smile"
    assert loaded["targets"][0]["shape_name"] == "Head"


def test_export_morph_subcommand_writes_scene_morph_metadata_as_binary_tri(tmp_path):
    input_path = tmp_path / "scene.json"
    output_path = tmp_path / "head.tri"
    input_path.write_text(
        json.dumps(
            {
                "game": "fo4",
                "root_nodes": [
                    {
                        "id": "head-node",
                        "type": "mesh",
                        "name": "Head",
                        "mesh": {
                            "vertices": [
                                {"x": 0.0, "y": 0.0, "z": 0.0},
                                {"x": 1.0, "y": 0.0, "z": 0.0},
                                {"x": 0.0, "y": 1.0, "z": 0.0},
                            ],
                            "triangles": [
                                {"v1": 0, "v2": 1, "v3": 2},
                            ],
                            "uvs": [
                                {"u": 0.0, "v": 0.0},
                                {"u": 1.0, "v": 0.0},
                                {"u": 0.0, "v": 1.0},
                            ],
                        },
                        "metadata": {
                            "morph": {
                                "sidecar_kind": "tri",
                                "channels": [
                                    {
                                        "name": "Smile",
                                        "group": "expression",
                                        "value": 0.0,
                                        "enabled": True,
                                    }
                                ],
                                "targets": [
                                    {
                                        "name": "Smile",
                                        "shape_name": "Head",
                                        "offsets": [
                                            {
                                                "vertex": 1,
                                                "x": 0.125,
                                                "y": 0.0,
                                                "z": 0.0,
                                            }
                                        ],
                                    }
                                ],
                            }
                        },
                        "children": [],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "export-morph",
            "--input-json", str(input_path),
            "--output", str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert not output_path.read_bytes().lstrip().startswith(b"{")
    loaded = build_morph_document(output_path, known_shape_names=["Head"])
    assert loaded["sidecar_kind"] == "tri"
    assert loaded["base_mesh"]["faces"] == [[0, 1, 2]]
    assert loaded["targets"][0]["name"] == "Smile"
    assert loaded["targets"][0]["offsets"][0]["vertex"] == 1


def test_export_morph_subcommand_fails_without_scene_morph_data(tmp_path):
    input_path = tmp_path / "scene.json"
    output_path = tmp_path / "empty.tri"
    input_path.write_text(
        json.dumps(
            {
                "game": "fo4",
                "root_nodes": [
                    {
                        "id": "mesh",
                        "type": "mesh",
                        "name": "Head",
                        "mesh": {
                            "vertices": [
                                {"x": 0.0, "y": 0.0, "z": 0.0},
                                {"x": 1.0, "y": 0.0, "z": 0.0},
                                {"x": 0.0, "y": 1.0, "z": 0.0},
                            ],
                            "triangles": [{"v1": 0, "v2": 1, "v3": 2}],
                        },
                        "metadata": {},
                        "children": [],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "export-morph",
            "--input-json", str(input_path),
            "--output", str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "morph" in result.stderr.lower()
    assert not output_path.exists()


def test_export_morph_subcommand_fails_for_empty_morph_document(tmp_path):
    input_path = tmp_path / "empty_morph.json"
    output_path = tmp_path / "empty.tri"
    input_path.write_text(
        json.dumps({"kind": "max_morph_document", "targets": []}),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "export-morph",
            "--input-json", str(input_path),
            "--output", str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )

    assert result.returncode != 0
    assert "morph" in result.stderr.lower()
    assert not output_path.exists()


def test_import_kf_subcommand_runs_through_module_cli(tmp_path):
    input_path = tmp_path / "idle.kf"
    output_path = tmp_path / "idle.json"
    input_path.write_bytes(b"test kf input")

    env = os.environ.copy()
    env[KF_TEST_HOOK_ENV] = "1"
    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "import-kf",
            "--input",
            str(input_path),
            "--output-json",
            str(output_path),
        ],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert payload["kind"] == "kf_animation_document"
    assert payload["name"] == "Idle"
    assert payload["source_path"] == str(input_path)
    assert payload["channels"][0]["bone_name"] == "Root"


def test_import_kf_subcommand_reads_real_binary_kf(tmp_path):
    input_path = tmp_path / "idle.kf"
    output_path = tmp_path / "idle.json"
    write_kf(
        AnimationClip(
            name="Idle",
            duration=1.0,
            cycle_type="clamp",
            accum_root="Root",
            channels=(
                BoneChannel(
                    bone_name="Root",
                    translations=(
                        AnimationKeyframe(time=0.0, value=(1.0, 2.0, 3.0)),
                    ),
                ),
            ),
        ),
        input_path,
        game="fnv",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "import-kf",
            "--input",
            str(input_path),
            "--output-json",
            str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert payload["kind"] == "kf_animation_document"
    assert payload["name"] == "Idle"
    assert payload["channels"][0]["bone_name"] == "Root"


def test_emit_kf_script_subcommand_writes_import_script(tmp_path):
    input_path = tmp_path / "idle.json"
    output_path = tmp_path / "idle.ms"
    input_path.write_text(
        json.dumps(
            {
                "kind": "kf_animation_document",
                "name": "Idle",
                "duration": 0.0,
                "channels": [{"bone_name": "Root", "translations": []}],
            }
        ),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "emit-kf-script",
            "--input-json",
            str(input_path),
            "--output-script",
            str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    script = output_path.read_text(encoding="utf-8")
    assert 'point name:"MB21_KF_Idle"' in script
    assert '"mb21_kf_bone_name" "Root"' in script


def test_export_kf_subcommand_runs_through_module_cli(tmp_path):
    input_path = tmp_path / "idle.json"
    output_path = tmp_path / "idle.kf"
    input_path.write_text(
        json.dumps(
            {
                "kind": "kf_animation_document",
                "name": "Idle",
                "duration": 1.0,
                "cycle_type": "clamp",
                "channels": [
                    {
                        "bone_name": "Root",
                        "priority": 26,
                        "translations": [
                            {"time": 0.0, "value": [1.0, 2.0, 3.0]},
                        ],
                        "rotations": [],
                        "scales": [],
                    }
                ],
            }
        ),
        encoding="utf-8",
    )

    env = os.environ.copy()
    env[KF_TEST_HOOK_ENV] = "1"
    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "export-kf",
            "--input-json",
            str(input_path),
            "--output",
            str(output_path),
        ],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert output_path.read_text(encoding="utf-8") == "test kf export:Idle:Root\n"


def test_export_kf_subcommand_writes_real_binary_kf(tmp_path):
    input_path = tmp_path / "idle.json"
    output_path = tmp_path / "idle.kf"
    input_path.write_text(
        json.dumps(
            {
                "kind": "kf_animation_document",
                "name": "Idle",
                "duration": 1.0,
                "cycle_type": "clamp",
                "accum_root": "Root",
                "channels": [
                    {
                        "bone_name": "Root",
                        "priority": 26,
                        "translations": [
                            {"time": 0.0, "value": [1.0, 2.0, 3.0]},
                            {"time": 1.0, "value": [4.0, 5.0, 6.0]},
                        ],
                        "rotations": [
                            {"time": 0.0, "value": [0.0, 0.0, 0.0, 1.0]},
                        ],
                        "scales": [],
                    }
                ],
                "events": [{"time": 0.0, "text": "start"}],
            }
        ),
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            sys.executable, "-m",
            "plugins.max_nif_plugin.runtime.backend_host",
            "export-kf",
            "--input-json",
            str(input_path),
            "--output",
            str(output_path),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    assert not output_path.read_bytes().lstrip().startswith(b"{")
    clip = read_kf(output_path)
    assert clip.name == "Idle"
    assert clip.accum_root == "Root"
    assert clip.channels[0].bone_name == "Root"
    assert clip.channels[0].translations[-1].value == (4.0, 5.0, 6.0)
