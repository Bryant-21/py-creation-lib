"""FO76 BGEM round-trip, glass-refraction, and downgrade tests.

``bgem_bin`` reads the v21+ glass refraction block and the v22-only
``GlassBlurScaleFactor``, as libfo76utils ``bgsmfile.cpp::loadBGEMFile`` does.
Covers byte-identical round-trip of a real FO76 v22 BGEM (glass off), round-trip
of a synthesized v22 BGEM with glass on, and ``convert.downgrade_bgem`` to
``BGEM_VERSION_FO4`` with FO76-only fields cleared and the result re-parseable.
"""

from __future__ import annotations

import io
from pathlib import Path

import pytest

from creation_lib.material_tools.bgem_bin import read_bgem
from creation_lib.material_tools.convert import BGEM_VERSION_FO4, downgrade_bgem
from creation_lib.material_tools.fo76_downgrade_policy import FO76_ONLY_BGEM_FIELDS

FIXTURE_DIR = (
    Path(__file__).parent.parent.parent
    / "conversion"
    / "tests"
    / "fixtures"
    / "fo76"
    / "materials"
)
FIXTURE = FIXTURE_DIR / "sample_v22.bgem"
GLASS_FIXTURE = FIXTURE_DIR / "sample_v22_glass.bgem"


@pytest.mark.skipif(not FIXTURE.exists(), reason="FO76 BGEM fixture missing")
def test_fo76_bgem_roundtrip_byte_identical():
    original = FIXTURE.read_bytes()
    data = read_bgem(io.BytesIO(original))
    assert data.header.version >= 20
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == original


@pytest.mark.skipif(not GLASS_FIXTURE.exists(), reason="FO76 glass BGEM fixture missing")
def test_fo76_bgem_glass_enabled_roundtrip():
    """The v21+ glass refraction block and v22 GlassBlurScaleFactor must
    survive a write/read cycle."""
    original = GLASS_FIXTURE.read_bytes()
    data = read_bgem(io.BytesIO(original))
    assert data.header.version == 22
    assert data.GlassEnabled is True
    assert data.GlassFresnelColor is not None
    assert data.GlassBlurScaleBase is not None
    assert data.GlassBlurScaleFactor is not None  # v22-only
    assert data.GlassRefractionScaleBase is not None
    # Round-trip byte-identical so the v22-only field ordering is locked in.
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == original


@pytest.mark.skipif(not FIXTURE.exists(), reason="FO76 BGEM fixture missing")
def test_fo76_bgem_downgrade_to_fo4_valid():
    data = read_bgem(io.BytesIO(FIXTURE.read_bytes()))
    fo4 = downgrade_bgem(data, BGEM_VERSION_FO4)
    assert fo4.header.version == BGEM_VERSION_FO4
    for name in FO76_ONLY_BGEM_FIELDS:
        assert getattr(fo4, name) is None, f"{name} not cleared by downgrade"
    # Downgraded instance must serialize to a valid v20 BGEM.
    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgem(buf)
    assert reloaded.header.version == BGEM_VERSION_FO4


@pytest.mark.skipif(not GLASS_FIXTURE.exists(), reason="FO76 glass BGEM fixture missing")
def test_fo76_bgem_glass_downgrade_clears_glass_fields():
    """Downgrading a glass-enabled v22 BGEM must clear the v21+ glass block."""
    data = read_bgem(io.BytesIO(GLASS_FIXTURE.read_bytes()))
    assert data.GlassEnabled is True
    fo4 = downgrade_bgem(data, BGEM_VERSION_FO4)
    assert fo4.GlassEnabled is None
    assert fo4.GlassFresnelColor is None
    assert fo4.GlassBlurScaleFactor is None
    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgem(buf)
    assert reloaded.header.version == BGEM_VERSION_FO4
