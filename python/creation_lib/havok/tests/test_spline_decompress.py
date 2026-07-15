"""Tests for spline compressed animation decompression."""
import math
import os

import pytest

# Skip if extracted FO4 animations not available
FO4_ANIM_DIR = "extracted/fo4/meshes/actors/alien/animations"
HAS_FO4_ANIMS = os.path.isdir(FO4_ANIM_DIR)


@pytest.fixture
def combat_idle_clip():
    """Extract a known spline-compressed FO4 animation."""
    if not HAS_FO4_ANIMS:
        pytest.skip("FO4 extracted animations not available")
    from creation_lib.hkxpack import unpack_hkx_to_xml
    from creation_lib.havok.animation_reader import extract_clip

    xml_path = unpack_hkx_to_xml(
        os.path.join(FO4_ANIM_DIR, "combat_idle.hkx")
    )
    clip = extract_clip(xml_path)
    assert clip is not None
    return clip


@pytest.fixture
def dodge_left_clip():
    if not HAS_FO4_ANIMS:
        pytest.skip("FO4 extracted animations not available")
    from creation_lib.hkxpack import unpack_hkx_to_xml
    from creation_lib.havok.animation_reader import extract_clip

    xml_path = unpack_hkx_to_xml(
        os.path.join(FO4_ANIM_DIR, "dodgeleft.hkx")
    )
    clip = extract_clip(xml_path)
    assert clip is not None
    return clip


class TestSplineDecompression:
    """Test decompression of spline-compressed FO4 animations."""

    def test_has_channels(self, combat_idle_clip):
        assert len(combat_idle_clip.channels) == 83

    def test_frame_count(self, combat_idle_clip):
        ch = combat_idle_clip.channels[0]
        assert len(ch.rotations) == 101  # numFrames from the XML

    def test_duration(self, combat_idle_clip):
        assert abs(combat_idle_clip.duration - 3.333333) < 0.001

    def test_no_warnings(self, combat_idle_clip):
        assert len(combat_idle_clip.warnings) == 0

    def test_source_format(self, combat_idle_clip):
        assert combat_idle_clip.source_format == "hkx"

    def test_quaternions_normalized(self, combat_idle_clip):
        """All decompressed quaternions should be approximately unit length."""
        for ch in combat_idle_clip.channels[:10]:
            for kf in ch.rotations[:5]:
                q = kf.value
                length = math.sqrt(sum(c * c for c in q))
                assert abs(length - 1.0) < 0.05, (
                    f"Quaternion not normalized: {q}, length={length}"
                )

    def test_identity_root(self, combat_idle_clip):
        """Track 0 (root) should be identity rotation."""
        ch = combat_idle_clip.channels[0]
        q = ch.rotations[0].value
        # Should be close to (0, 0, 0, 1)
        assert abs(q[3]) > 0.99

    def test_translations_present(self, combat_idle_clip):
        """Should have translation keyframes on some tracks."""
        has_trans = sum(1 for ch in combat_idle_clip.channels if len(ch.translations) > 0)
        assert has_trans > 0

    def test_different_animation(self, dodge_left_clip):
        """A different animation should also decompress."""
        assert len(dodge_left_clip.channels) == 83
        ch = dodge_left_clip.channels[1]
        assert len(ch.rotations) > 0


class TestBatchDecompression:
    """Test decompression across multiple files."""

    @pytest.mark.skipif(not HAS_FO4_ANIMS, reason="FO4 anims not available")
    def test_all_alien_animations(self):
        from creation_lib.hkxpack import unpack_hkx_to_xml
        from creation_lib.havok.animation_reader import extract_clip

        success = 0
        fail = 0
        for f in os.listdir(FO4_ANIM_DIR):
            if not f.endswith(".hkx"):
                continue
            try:
                xml_path = unpack_hkx_to_xml(os.path.join(FO4_ANIM_DIR, f))
                clip = extract_clip(xml_path)
                if clip and clip.channels:
                    success += 1
                else:
                    fail += 1
            except Exception:
                fail += 1
        assert fail == 0, f"{fail} animations failed out of {success + fail}"
        assert success > 20  # alien has ~27 animations

