from __future__ import annotations

from pathlib import Path

import pytest

from creation_lib.material_tools import upgrade_v22
from creation_lib.material_tools.bgsm_bin import read_bgsm

from .test_convert_py_regression import _default_bgsm_v20

try:
    upgrade_v22._native()
except RuntimeError:  # pragma: no cover - only without a rebuilt native extension
    pytest.skip("material_upgrade_native not built", allow_module_level=True)


def _write_v2(path: Path, diffuse: str, spec: str) -> None:
    data = _default_bgsm_v20()
    data.header.version = 2
    data.header.env_mapping = False
    data.header.env_mapping_mask_scale = 1.0
    data.header.depth_bias = None
    data.header.mask_writes = None
    data.DiffuseTexture = diffuse
    data.NormalTexture = diffuse.replace("_d.dds", "_n.dds")
    data.SmoothSpecTexture = spec
    data.SpecularTexture = None
    data.LightingTexture = None
    data.PBR = None
    data.RootMaterialPath = ""
    for name in ("Translucency", "TranslucencyThickObject", "TranslucencyMixAlbedoWithSubsurfaceColor",
                 "TranslucencySubsurfaceColor", "TranslucencyTransmissiveScale", "TranslucencyTurbulence",
                 "CustomPorosity", "PorosityValue", "LumEmittance", "UseAdaptativeEmissive",
                 "AdaptativeEmissive_ExposureOffset", "AdaptativeEmissive_FinalExposureMin",
                 "AdaptativeEmissive_FinalExposureMax", "Terrain", "FlowTexture", "DistanceFieldAlphaTexture"):
        setattr(data, name, None)
    for name in ("RimLighting", "SubsurfaceLighting", "BackLighting", "EnvironmentMappingWindow", "EnvironmentMappingEye"):
        setattr(data, name, False)
    for name in ("RimPower", "BackLightPower", "SubsurfaceLightingRolloff", "WetnessControlEnvMapScale",
                 "DisplacementTextureBias", "DisplacementTextureScale", "TessellationPnScale",
                 "TessellationBaseFactor", "TessellationFadeDistance"):
        setattr(data, name, 0.0)
    for name in ("EnvmapTexture", "InnerLayerTexture", "DisplacementTexture"):
        setattr(data, name, "")
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as handle:
        data.write(handle)


@pytest.fixture()
def data_dir(tmp_path: Path) -> Path:
    from creation_lib.dds.native_runtime import write_dds_rgba

    data = tmp_path / "Data"
    _write_v2(data / "Materials/Props/Can.bgsm", "Props/Can_d.dds", "Props/Can_s.dds")
    _write_v2(data / "Materials/Props/Box.bgsm", "Props/Box_d.dds", "Props/Box_s.dds")
    spec = bytes(b for i in range(256) for b in (200, i, 0, 255))
    (data / "Textures/Props").mkdir(parents=True)
    for name in ("Can_s.dds", "Box_s.dds"):
        assert write_dds_rgba(str(data / "Textures/Props" / name), 16, 16, spec, format="BC5_UNORM", generate_mips=True, use_gpu=False)
    return data


def test_plan_is_a_dry_run(data_dir: Path, tmp_path: Path):
    opts = upgrade_v22.UpgradeOptions(sources=[str(data_dir)], output=str(tmp_path / "out"))
    report = upgrade_v22.plan(opts)
    assert report["dry_run"] is True
    assert report["summary"]["upgrade"] == 2
    assert "textures/materialupgrade/dielectric_r.dds" in report["texture_outputs"]
    assert not (tmp_path / "out").exists()


def test_run_writes_v22_materials_and_report(data_dir: Path, tmp_path: Path):
    out = tmp_path / "out"
    calls: list[tuple[int, int, str]] = []
    report = upgrade_v22.run(
        upgrade_v22.UpgradeOptions(sources=[str(data_dir)], output=str(out)),
        progress=lambda done, total, msg: calls.append((done, total, msg)),
    )
    assert report["result"]["errors"] == []
    with (out / "materials/props/can.bgsm").open("rb") as handle:
        upgraded = read_bgsm(handle)
    assert upgraded.header.version == 22 and upgraded.PBR
    assert upgraded.LightingTexture.rstrip("\0") == "props/can_l.dds"
    assert (out / "textures/props/can_l.dds").is_file()
    assert (out / "materialupgrade/upgrade_report.json").is_file()
    assert calls and calls[-1][2] == "done"


def test_replace_set_is_excluded_from_upgrade(data_dir: Path, tmp_path: Path):
    opts = upgrade_v22.UpgradeOptions(
        sources=[str(data_dir)], output=str(tmp_path / "out"), exclude_paths=["materials/props/can.bgsm"]
    )
    assert upgrade_v22.selected_keys(opts) == ["materials/props/box.bgsm"]
