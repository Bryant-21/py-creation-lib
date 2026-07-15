import pytest

from creation_lib.havok.parsers.animation import AnimationData, parse_animation


SAMPLE_LOSSLESS_ANIM_XML = """\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkaAnimationContainer" signature="0x8dc20f3">
      <hkparam name="skeletons" numelements="0"></hkparam>
      <hkparam name="animations" numelements="1">
        <hkobject>#0002</hkobject>
      </hkparam>
      <hkparam name="bindings" numelements="0"></hkparam>
    </hkobject>
    <hkobject name="#0002" class="hkaLosslessCompressedAnimation" signature="0x0">
      <hkparam name="duration">1.000000</hkparam>
      <hkparam name="numberOfTransformTracks">2</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="staticRotations" numelements="2">
        (0.000000 0.000000 0.000000 1.000000)
        (0.000000 0.707107 0.000000 0.707107)
      </hkparam>
      <hkparam name="staticTranslations" numelements="2">
        (0.000000 0.000000 0.000000)
        (1.000000 0.000000 0.000000)
      </hkparam>
      <hkparam name="rotationTypeAndOffsets" numelements="2">1 5</hkparam>
      <hkparam name="translationTypeAndOffsets" numelements="2">1 5</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>
"""


class TestParseAnimation:
    def test_extracts_duration(self, tmp_path):
        xml_path = tmp_path / "Anim.xml"
        xml_path.write_text(SAMPLE_LOSSLESS_ANIM_XML)
        result = parse_animation(xml_path)
        assert result.duration == pytest.approx(1.0)

    def test_extracts_bone_count(self, tmp_path):
        xml_path = tmp_path / "Anim.xml"
        xml_path.write_text(SAMPLE_LOSSLESS_ANIM_XML)
        result = parse_animation(xml_path)
        assert result.bone_count == 2

    def test_detects_lossless_compression(self, tmp_path):
        xml_path = tmp_path / "Anim.xml"
        xml_path.write_text(SAMPLE_LOSSLESS_ANIM_XML)
        result = parse_animation(xml_path)
        assert result.compression_type == "lossless"

    def test_extracts_frame0_transforms(self, tmp_path):
        xml_path = tmp_path / "Anim.xml"
        xml_path.write_text(SAMPLE_LOSSLESS_ANIM_XML)
        result = parse_animation(xml_path)
        assert result.frame0_transforms is not None
        assert len(result.frame0_transforms) > 0  # Binary BLOB

    def test_handles_empty_animation(self, tmp_path):
        xml_path = tmp_path / "Empty.xml"
        xml_path.write_text('<?xml version="1.0"?><hkpackfile><hksection name="__data__"></hksection></hkpackfile>')
        result = parse_animation(xml_path)
        assert result.bone_count == 0
        assert result.compression_type == "unknown"
