"""Regression tests for ``creation_lib.material_tools.convert`` ground-truth downgrade.

``convert.downgrade_bgsm`` / ``convert.downgrade_bgem`` are the
battle-tested FO76 -> FO4 material downgrade implementations used by
production BACUP callers,
``ui/tools/conversion/nif_converter.py``). The bugs below are real
incidents that the inline comments in ``convert.py`` document; the tests
here lock the fixes in so a future refactor cannot silently regress them.

The tests mutate a real FO76 v22 BGSM / BGEM fixture so we don't have to
construct a valid dataclass from scratch — the fixtures live at
``bacup/py_bacup_lib/python/bacup_lib/tests/fixtures/fo76/materials/``.
"""
from __future__ import annotations

import copy
import io
from pathlib import Path

import pytest

from creation_lib.material_tools.bgsm_bin import BGSMData, read_bgsm
from creation_lib.material_tools.bgem_bin import BGEMData, read_bgem
from creation_lib.material_tools.convert import (
    BGSM_VERSION_FO4,
    BGEM_VERSION_FO4,
    downgrade_bgsm,
    downgrade_bgem,
)

FIXTURE_DIR = (
    Path(__file__).parent.parent.parent
    / "conversion"
    / "tests"
    / "fixtures"
    / "fo76"
    / "materials"
)
BGSM_FIXTURE = FIXTURE_DIR / "sample_v22.bgsm"
BGEM_FIXTURE = FIXTURE_DIR / "sample_v22.bgem"
BGEM_GLASS_FIXTURE = FIXTURE_DIR / "sample_v22_glass.bgem"


pytestmark = pytest.mark.skipif(
    not BGSM_FIXTURE.exists() or not BGEM_FIXTURE.exists(),
    reason="FO76 BGSM/BGEM fixtures missing",
)


def _load_bgsm_v20() -> BGSMData:
    """Load the v22 fixture and force-clamp its header to v20 so the
    downgrade path sees a cleanly v20 input. The fixture's v22 layout
    is byte-compatible with v20 for the fields we touch (there are no
    v21/v22-only BGSM-level fields in the current dataclass). Cloning
    avoids cross-test mutation."""
    data = read_bgsm(io.BytesIO(BGSM_FIXTURE.read_bytes()))
    clone = copy.deepcopy(data)
    clone.header.version = 20
    return clone


def _load_bgem_v22() -> BGEMData:
    return read_bgem(io.BytesIO(BGEM_FIXTURE.read_bytes()))


def _load_bgem_v22_glass() -> BGEMData:
    return read_bgem(io.BytesIO(BGEM_GLASS_FIXTURE.read_bytes()))


# ---------------------------------------------------------------------------
# 1. Mirror-shiny weapon bug
# ---------------------------------------------------------------------------


def test_mirror_shiny_weapon_bug_specular_promoted_to_smoothspec():
    """FO76 v20 BGSM with a populated SpecularTexture and an empty
    SmoothSpecTexture must NOT write the _s.dds roughness map into
    EnvmapTexture / InnerLayerTexture / DisplacementTexture — the FO4
    render path samples those as a cubemap and produces mirror-shiny
    weapons. The _s.dds must instead be promoted into SmoothSpecTexture.

    EnvmapTexture is now populated with a heuristic FO4 cubemap (not the
    _s.dds) — the cubemap injection lives in select_cubemap, exercised
    separately in test_cubemap_heuristics.
    """
    data = _load_bgsm_v20()
    data.SpecularTexture = "weapons/gaussrifle/foo_s.dds"
    data.SmoothSpecTexture = ""
    data.LightingTexture = ""
    data.FlowTexture = ""

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    # The _s.dds roughness map MUST NOT leak into any FO4 cubemap slot.
    assert "_s.dds" not in (fo4.EnvmapTexture or "")
    assert fo4.InnerLayerTexture == ""
    assert fo4.DisplacementTexture == ""
    assert fo4.SmoothSpecTexture == "weapons/gaussrifle/foo_s.dds"
    # EnvmapTexture should be a real FO4 cubemap (heuristic-injected).
    assert fo4.EnvmapTexture and "Cubemaps" in fo4.EnvmapTexture
    assert fo4.header.env_mapping is True


# ---------------------------------------------------------------------------
# 2/3. Whole-object BGSM emittance suppression
# ---------------------------------------------------------------------------


def test_lighting_texture_emittance_disabled_for_static_fo4_bgsm():
    """FO4 applies converted BGSM emittance across the whole object surface."""
    data = _load_bgsm_v20()
    data.LightingTexture = "weapons/gaussrifle/foo_l.dds"
    data.GlowTexture = ""
    data.EmitEnabled = True
    data.EmittanceMult = 10.0
    if data.EmittanceColor is None:
        data.EmittanceColor = (1.0, 1.0, 0.0)

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    assert not fo4.GlowTexture
    assert fo4.Glowmap is False
    assert fo4.EmitEnabled is False
    assert fo4.EmittanceColor is None
    assert fo4.EmittanceMult == 1.0
    assert fo4.LightingTexture is None


def test_lighting_texture_emittance_preserved_for_effect_bgsm():
    data = _load_bgsm_v20()
    data.LightingTexture = "effects/foo_l.dds"
    data.GlowTexture = ""
    data.EmitEnabled = True
    data.EmittanceMult = 10.0

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="materials/effects/foo.bgsm")

    assert fo4.GlowTexture == "effects/foo_l.dds"
    assert fo4.Glowmap is True
    assert fo4.EmitEnabled is True
    assert fo4.EmittanceMult == 1.0
    assert fo4.LightingTexture is None


def test_lighting_texture_not_promoted_when_emit_disabled():
    """Without EmitEnabled, LightingTexture must not produce a FO4 glow path."""
    data = _load_bgsm_v20()
    data.LightingTexture = "weapons/gaussrifle/foo_l.dds"
    data.GlowTexture = ""
    data.EmitEnabled = False

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    assert not fo4.GlowTexture
    assert fo4.LightingTexture is None


# ---------------------------------------------------------------------------
# 4. Translucency -> SubsurfaceLighting value preservation
# ---------------------------------------------------------------------------


def test_translucency_converted_to_subsurface_lighting():
    """FO76 v20 BGSM with Translucency=True and TranslucencyTransmissiveScale
    must downgrade to SubsurfaceLighting=True with matching rolloff; the
    entire Translucency* block must be cleared."""
    data = _load_bgsm_v20()
    data.Translucency = True
    data.TranslucencyTransmissiveScale = 0.5
    data.TranslucencyThickObject = True
    data.TranslucencyMixAlbedoWithSubsurfaceColor = True
    data.TranslucencySubsurfaceColor = (0.8, 0.4, 0.2)
    data.TranslucencyTurbulence = 0.25

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    assert fo4.SubsurfaceLighting is True
    assert fo4.SubsurfaceLightingRolloff == pytest.approx(0.5)
    assert fo4.Translucency is None
    assert fo4.TranslucencyThickObject is None
    assert fo4.TranslucencyMixAlbedoWithSubsurfaceColor is None
    assert fo4.TranslucencySubsurfaceColor is None
    assert fo4.TranslucencyTransmissiveScale is None
    assert fo4.TranslucencyTurbulence is None


# ---------------------------------------------------------------------------
# 5. RootMaterialPath synthesis
# ---------------------------------------------------------------------------


def test_root_material_path_synthesized_from_source_path():
    """FO76 ships RootMaterialPath empty on 99% of BGSMs; FO4 vanilla
    relies on it for per-category shader param inheritance. When the
    source has empty RootMaterialPath, downgrade_bgsm must synthesize a
    plausible template via templates.resolve_root_material_path(source_path)."""
    data = _load_bgsm_v20()
    data.RootMaterialPath = ""

    fo4 = downgrade_bgsm(
        data,
        BGSM_VERSION_FO4,
        source_path="weapons/gaussrifle/foo.bgsm",
    )

    assert fo4.RootMaterialPath
    assert "template/" in fo4.RootMaterialPath.lower()


def test_vegetation_material_defaults_for_leaf_template():
    data = _load_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = True
    data.Translucency = True
    data.TranslucencyTransmissiveScale = 1.0
    data.SpecularTexture = "Landscape/Plants/Bramble01_r.dds"
    data.LightingTexture = "Landscape/Plants/Bramble01_l.dds"

    fo4 = downgrade_bgsm(
        data,
        BGSM_VERSION_FO4,
        source_path="materials/landscape/plants/bramble.bgsm",
    )

    assert fo4.RootMaterialPath == "Template/LeafTemplate_Wet.bgsm"
    assert fo4.BackLighting is True
    assert fo4.BackLightPower == pytest.approx(0.25)
    assert fo4.SubsurfaceLighting is True
    assert fo4.SubsurfaceLightingRolloff == pytest.approx(2.0)
    assert fo4.EnvmapTexture == ""
    assert not fo4.GlowTexture
    assert fo4.SmoothSpecTexture == "Landscape/Plants/Bramble01_s.dds"


def test_grass_material_uses_grass_template_before_tree_flag():
    data = _load_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = True
    data.Translucency = True

    fo4 = downgrade_bgsm(
        data,
        BGSM_VERSION_FO4,
        source_path="materials/landscape/grass/mtntop_grass01.bgsm",
    )

    assert fo4.RootMaterialPath == "Template/GrassTemplate_Wet.BGSM"
    assert fo4.SubsurfaceLighting is True
    assert fo4.EnvmapTexture == ""


def test_landscape_rocks_material_uses_rock_template():
    data = _load_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = False

    fo4 = downgrade_bgsm(
        data,
        BGSM_VERSION_FO4,
        source_path="materials/landscape/rocks/mtntopcliff01.bgsm",
    )

    assert fo4.RootMaterialPath == "template/RockTemplate_Wet.bgsm"


@pytest.mark.parametrize(
    ("source_path", "expected_template"),
    [
        ("materials/landscape/rocks/rockslab01.bgsm", "template/RockSlabTemplate_Wet.bgsm"),
        ("materials/landscape/ground/crackedmud01.bgsm", "template/CrackedMudTemplate_Wet.bgsm"),
        ("materials/landscape/roads/asphaltroad01.bgsm", "template/AsphaltTemplate_Wet.bgsm"),
        ("materials/setdressing/rubbertire01.bgsm", "template/RubberTemplate_Wet.bgsm"),
        ("materials/setdressing/hides/radstaghides.bgsm", "template/ClothTemplate_Felt_Wet.bgsm"),
        ("materials/architecture/buildings/metalrailing01.bgsm", "template/WroughtIronMetalTemplate_Wet.bgsm"),
        ("materials/architecture/capsules/capdetailsheet01.bgsm", "template/CapMetalTemplate_Wet.bgsm"),
        ("materials/vehicles/bus/bus01.bgsm", "template/VehicleBusTemplate_Wet.bgsm"),
        ("materials/vehicles/car/car01.bgsm", "template/VehicleTemplate_Wet.bgsm"),
        ("materials/actors/dogmeat/dogmeat_body.bgsm", "template/FurTemplate_Wet.bgsm"),
        ("materials/gore/goreorgans.bgsm", "template/basicsmooth.bgsm"),
        ("materials/gore/goresupermutanthead.bgsm", "template/SkinTemplate_Wet.bgsm"),
    ],
)
def test_root_material_path_uses_fo4_template_family_rules(source_path, expected_template):
    data = _load_bgsm_v20()
    data.RootMaterialPath = ""
    data.Tree = False

    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path=source_path)

    assert fo4.RootMaterialPath == expected_template


# ---------------------------------------------------------------------------
# 6. Round-trip validity of downgraded BGSM
# ---------------------------------------------------------------------------


def test_downgraded_bgsm_round_trips_through_reader():
    """A downgraded BGSM must serialize + re-parse cleanly as a valid
    FO4 v2 BGSM."""
    data = _load_bgsm_v20()
    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/gaussrifle/foo.bgsm")

    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgsm(buf)

    assert reloaded.header.version == BGSM_VERSION_FO4
    assert reloaded.DiffuseTexture == fo4.DiffuseTexture
    assert reloaded.NormalTexture == fo4.NormalTexture


# ---------------------------------------------------------------------------
# 7. BGEM Glass field clearing
# ---------------------------------------------------------------------------


@pytest.mark.skipif(
    not BGEM_GLASS_FIXTURE.exists(),
    reason="FO76 glass BGEM fixture missing",
)
def test_bgem_glass_fields_cleared_on_downgrade():
    """FO76 v22 BGEM with Glass* fields populated must downgrade with
    all Glass* fields cleared so the FO4 v20 writer doesn't see stray
    glass state."""
    data = _load_bgem_v22_glass()
    assert data.GlassEnabled is True

    fo4 = downgrade_bgem(data, BGEM_VERSION_FO4)

    assert fo4.GlassRoughnessScratch is None
    assert fo4.GlassDirtOverlay is None
    assert fo4.GlassEnabled is None
    assert fo4.GlassFresnelColor is None
    assert fo4.GlassBlurScaleBase is None
    assert fo4.GlassBlurScaleFactor is None
    assert fo4.GlassRefractionScaleBase is None

    # Round-trip as a valid v20 BGEM.
    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgem(buf)
    assert reloaded.header.version == BGEM_VERSION_FO4


# ---------------------------------------------------------------------------
# 8. BGEM no-op when already at target
# ---------------------------------------------------------------------------


def test_bgem_downgrade_noop_when_already_at_target():
    """Passing a BGEM whose header is already at the target version must
    return it unchanged (the same instance, per current contract)."""
    data = _load_bgem_v22()
    data.header.version = BGEM_VERSION_FO4  # simulate "already FO4"

    result = downgrade_bgem(data, BGEM_VERSION_FO4)

    assert result is data
    assert result.header.version == BGEM_VERSION_FO4
