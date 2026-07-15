import json
from pathlib import Path

import numpy as np

import scripts.compare_fo76_texture_conversion as compare_script


def _touch(path):
    path.write_bytes(b"")


def test_discover_bundles_returns_complete_drl_sets(tmp_path):
    _touch(tmp_path / "part_b_d.dds")
    _touch(tmp_path / "part_b_r.dds")
    _touch(tmp_path / "part_b_l.dds")
    _touch(tmp_path / "part_a_d.dds")
    _touch(tmp_path / "part_a_r.dds")
    _touch(tmp_path / "part_a_l.dds")
    _touch(tmp_path / "missing_l_d.dds")
    _touch(tmp_path / "missing_l_r.dds")
    _touch(tmp_path / "part_a_n.dds")

    bundles = compare_script.discover_bundles(tmp_path)

    assert [bundle.name for bundle in bundles] == ["part_a", "part_b"]
    assert bundles[0].diffuse_path == tmp_path / "part_a_d.dds"
    assert bundles[0].reflectivity_path == tmp_path / "part_a_r.dds"
    assert bundles[0].lighting_path == tmp_path / "part_a_l.dds"


def test_compare_folder_writes_summary_for_each_discovered_bundle(tmp_path, monkeypatch):
    _touch(tmp_path / "part_d.dds")
    _touch(tmp_path / "part_r.dds")
    _touch(tmp_path / "part_l.dds")
    out_dir = tmp_path / "out"
    calls = []

    def fake_compare(diffuse_path, reflectivity_path, lighting_path, bundle_out_dir):
        calls.append((diffuse_path, reflectivity_path, lighting_path, bundle_out_dir))
        return {"native_vs_legacy_specgloss": {"mae": 0.25}}

    monkeypatch.setattr(compare_script, "compare", fake_compare)

    summary = compare_script.compare_folder(tmp_path, out_dir)

    assert calls == [
        (
            tmp_path / "part_d.dds",
            tmp_path / "part_r.dds",
            tmp_path / "part_l.dds",
            out_dir / "part",
        )
    ]
    assert summary == {"part": {"native_vs_legacy_specgloss": {"mae": 0.25}}}
    assert json.loads((out_dir / "summary.json").read_text(encoding="utf-8")) == summary


def test_compare_writes_all_method_outputs(tmp_path, monkeypatch):
    diffuse_path = tmp_path / "part_d.dds"
    reflectivity_path = tmp_path / "part_r.dds"
    lighting_path = tmp_path / "part_l.dds"
    for path in (diffuse_path, reflectivity_path, lighting_path):
        _touch(path)

    arrays = {
        diffuse_path: np.full((1, 1, 4), [0.2, 0.3, 0.4, 1.0], dtype=np.float32),
        reflectivity_path: np.full((1, 1, 4), [0.5, 0.5, 0.5, 1.0], dtype=np.float32),
        lighting_path: np.full((1, 1, 4), [0.6, 0.7, 0.0, 0.8], dtype=np.float32),
        "directx_d.dds": np.full((1, 1, 4), [0.21, 0.31, 0.41, 1.0], dtype=np.float32),
        "directx_s.dds": np.full((1, 1, 4), [0.22, 0.62, 0.0, 1.0], dtype=np.float32),
        "directx_g.dds": np.full((1, 1, 4), [0.8, 0.8, 0.8, 1.0], dtype=np.float32),
    }

    def fake_load_texture(path):
        path = Path(path)
        if path in arrays:
            return arrays[path]
        return arrays[path.name]

    def fake_directx(*args, **kwargs):
        return True

    saved = []

    def fake_save_preview(arr, path):
        saved.append(path.name)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(str(float(arr.mean())), encoding="utf-8")

    monkeypatch.setattr(compare_script, "load_texture", fake_load_texture)
    monkeypatch.setattr(
        compare_script.dds_native_runtime,
        "remix_fo76_bundle_to_fo4",
        fake_directx,
    )
    monkeypatch.setattr(compare_script, "_save_preview", fake_save_preview)

    compare_script.compare(diffuse_path, reflectivity_path, lighting_path, tmp_path / "out")

    assert sorted(saved) == [
        "directx_diffuse.png",
        "directx_glow.png",
        "directx_specgloss.png",
        "legacy_diffuse.png",
        "legacy_glow.png",
        "legacy_specgloss.png",
        "materials_native_diffuse.png",
        "materials_native_glow.png",
        "materials_native_specgloss.png",
    ]
