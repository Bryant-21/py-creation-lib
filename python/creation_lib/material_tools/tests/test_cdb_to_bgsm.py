"""Tests for cdb_to_bgsm: CE2Material -> BGSMData translator.

These tests exercise the dataclass-level API of ``cdb_to_bgsm`` with
hand-constructed ``CE2Material`` fixtures. The full ComponentBlob ->
CE2Layer walker that reads out of a real ``MaterialsCDB`` is deferred
-- downstream orchestration feeds fixture BGSMs through the
orchestrator, not live .mat references.
"""
from __future__ import annotations

import logging

import pytest

from creation_lib.core.game_profiles import RemixProfile
from creation_lib.material_tools.bgsm_bin import BGSMData
from creation_lib.material_tools.cdb_to_bgsm import cdb_to_bgsm
from creation_lib.material_tools.materials_cdb import (
    CE2Blender,
    CE2Layer,
    CE2Material,
    CE2MaterialProps,
    CE2TextureSet,
    CE2UVStream,
)

REFERENCE_PROFILE = RemixProfile(ao_multiplier=0.5, spec_offset=0.8)


def _make_layer(
    smooth: float = 0.6,
    metal: float = 0.0,
    diff: str = "tex_d.dds",
    norm: str = "tex_n.dds",
) -> CE2Layer:
    return CE2Layer(
        material=CE2MaterialProps(smoothness=smooth, metalness=metal),
        texture_set=CE2TextureSet(diffuse=diff, normal=norm),
        uv_stream=CE2UVStream(),
    )


# ---------------------------------------------------------------------------
# Basic single-layer translation
# ---------------------------------------------------------------------------

def test_single_layer_roundtrip():
    import io

    mat = CE2Material(name="gun", layers=[_make_layer()])
    bgsm = cdb_to_bgsm(mat, target_version=22, remix_profile=RemixProfile())
    assert isinstance(bgsm, BGSMData)
    # Should land at FO76 version (caller is responsible for downgrade via
    # creation_lib.material_tools.convert.downgrade_bgsm afterwards).
    assert bgsm.header.version == 22
    assert bgsm.DiffuseTexture == "tex_d.dds"
    assert bgsm.NormalTexture == "tex_n.dds"
    # Smoothness is passed through (gloss_multiplier defaults to 1.0 in a
    # neutral RemixProfile, and roughness = 1 - 0.6 = 0.4 -> gloss = 0.6).
    assert 0.59 < bgsm.Smoothness < 0.61

    # The output must be a complete, serializable BGSMData (every required
    # field populated).
    buf = io.BytesIO()
    bgsm.write(buf)
    assert buf.tell() > 0

    # Emit as FO76 v20 (pre-DistanceFieldAlphaTexture) so the orchestrator
    # can choose the exact major version per source profile.
    bgsm_v20 = cdb_to_bgsm(mat, target_version=20, remix_profile=RemixProfile())
    assert bgsm_v20.header.version == 20


# ---------------------------------------------------------------------------
# Multi-layer flattening: top layer wins, blenders/LOD materials dropped
# ---------------------------------------------------------------------------

def test_multi_layer_logs_drops(caplog):
    caplog.set_level(logging.INFO)
    mat = CE2Material(
        name="multi",
        layers=[
            _make_layer(diff="a_d.dds", norm="a_n.dds"),
            _make_layer(diff="b_d.dds", norm="b_n.dds"),
        ],
        blenders=[CE2Blender()],
    )
    bgsm = cdb_to_bgsm(mat, target_version=22, remix_profile=RemixProfile())
    # Top (highest-index) layer wins.
    assert bgsm.DiffuseTexture == "b_d.dds"
    assert bgsm.NormalTexture == "b_n.dds"
    messages = " ".join(r.message for r in caplog.records)
    assert "flattening" in messages
    assert "blenders" in messages

    caplog.clear()
    lod = CE2Material(name="lod0", layers=[_make_layer(diff="lod_d.dds")])
    lod_mat = CE2Material(
        name="main",
        layers=[_make_layer(diff="main_d.dds")],
        lod_materials=[lod],
    )
    lod_bgsm = cdb_to_bgsm(lod_mat, target_version=22, remix_profile=RemixProfile())
    assert lod_bgsm.DiffuseTexture == "main_d.dds"
    assert any("LOD" in r.message for r in caplog.records)


# ---------------------------------------------------------------------------
# PBR -> spec-gloss: FO76 reference scalar behavior
# ---------------------------------------------------------------------------

def test_metallic_converts_to_specular():
    mat = CE2Material(
        name="steel",
        layers=[_make_layer(smooth=1.0, metal=1.0)],
    )
    bgsm = cdb_to_bgsm(mat, target_version=22, remix_profile=REFERENCE_PROFILE)
    assert bgsm.SpecularColor[0] > 0.9
    assert bgsm.SpecularColor[1] > 0.9
    assert bgsm.SpecularColor[2] > 0.9
    # Full smoothness (roughness=0).
    assert bgsm.Smoothness > 0.99

    # A dielectric (metal=0.0) instead uses the reference-profile specular
    # fill, rather than the metallic color-passthrough above.
    dielectric = CE2Material(
        name="plastic",
        layers=[_make_layer(smooth=0.5, metal=0.0)],
    )
    dielectric_bgsm = cdb_to_bgsm(dielectric, target_version=22, remix_profile=REFERENCE_PROFILE)
    assert dielectric_bgsm.SpecularColor[0] == pytest.approx(0.22, abs=1e-5)
    assert dielectric_bgsm.SpecularColor[0] == dielectric_bgsm.SpecularColor[1] == dielectric_bgsm.SpecularColor[2]


def test_remix_profile_gloss_multiplier_applied():
    profile = RemixProfile(gloss_multiplier=0.5, spec_offset=0.8)
    mat = CE2Material(
        name="half_gloss",
        layers=[_make_layer(smooth=1.0, metal=0.0)],
    )
    bgsm = cdb_to_bgsm(mat, target_version=22, remix_profile=profile)
    # roughness = 0 -> base gloss = 1, scaled by 0.5 -> 0.5.
    assert 0.49 < bgsm.Smoothness < 0.51


# ---------------------------------------------------------------------------
# Error cases
# ---------------------------------------------------------------------------

def test_empty_material_raises():
    mat = CE2Material(name="bad")  # no layers
    with pytest.raises(ValueError, match="has no layers"):
        cdb_to_bgsm(mat, target_version=22, remix_profile=RemixProfile())
