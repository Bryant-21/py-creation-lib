"""FO76 BGSM round-trip and downgrade tests.

fo76texconv's ``bgsm.cpp`` stops at version 17 fields, yet the reader consumes the
real FO76 v22 sample (``fixtures/fo76/materials/sample_v22.bgsm``) in full and
round-trips it byte-identical: the file has no v18-v22 BGSM-level fields.
"""

from __future__ import annotations

import io
from pathlib import Path

import pytest

from creation_lib.material_tools.bgsm_bin import read_bgsm
from creation_lib.material_tools.convert import BGSM_VERSION_FO4, downgrade_bgsm
from creation_lib.material_tools.fo76_downgrade_policy import FO76_ONLY_BGSM_FIELDS

FIXTURE = (
    Path(__file__).parent.parent.parent
    / "conversion"
    / "tests"
    / "fixtures"
    / "fo76"
    / "materials"
    / "sample_v22.bgsm"
)


@pytest.mark.skipif(not FIXTURE.exists(), reason="FO76 BGSM fixture missing")
def test_fo76_bgsm_roundtrip_byte_identical():
    original = FIXTURE.read_bytes()
    data = read_bgsm(io.BytesIO(original))
    assert data.header.version >= 20
    buf = io.BytesIO()
    data.write(buf)
    assert buf.getvalue() == original


@pytest.mark.skipif(not FIXTURE.exists(), reason="FO76 BGSM fixture missing")
def test_fo76_bgsm_downgrade_to_fo4_valid_v2():
    data = read_bgsm(io.BytesIO(FIXTURE.read_bytes()))
    fo4 = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/test/sample.bgsm")
    assert fo4.header.version == BGSM_VERSION_FO4
    # FO76-only fields (per shared policy) must be cleared.
    for name in FO76_ONLY_BGSM_FIELDS:
        assert getattr(fo4, name) is None, f"{name} not cleared by downgrade"
    # Original instance is untouched.
    assert data.header.version == 22
    # The downgraded data must serialize to a valid v2 BGSM that the reader
    # can re-parse without error.
    buf = io.BytesIO()
    fo4.write(buf)
    buf.seek(0)
    reloaded = read_bgsm(buf)
    assert reloaded.header.version == BGSM_VERSION_FO4


def test_downgrade_does_not_mutate_original():
    """Pure-construction smoke test, no fixture needed."""
    if not FIXTURE.exists():
        pytest.skip("FO76 BGSM fixture missing")
    data = read_bgsm(io.BytesIO(FIXTURE.read_bytes()))
    original_version = data.header.version
    _ = downgrade_bgsm(data, BGSM_VERSION_FO4, source_path="weapons/test/sample.bgsm")
    assert data.header.version == original_version
