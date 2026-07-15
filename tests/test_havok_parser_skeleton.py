import json
from creation_lib.havok.parsers.skeleton import SkeletonData, parse_skeleton


SAMPLE_SKELETON_XML = """\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0090" class="hkaSkeleton" signature="0x366e2b87">
      <hkparam name="name">FXSkeleton</hkparam>
      <hkparam name="parentIndices" numelements="3">
        -1
        0
        1
      </hkparam>
      <hkparam name="bones" numelements="3">
        <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">false</hkparam></hkobject>
        <hkobject><hkparam name="name">Bone01</hkparam><hkparam name="lockTranslation">false</hkparam></hkobject>
        <hkobject><hkparam name="name">Bone02</hkparam><hkparam name="lockTranslation">false</hkparam></hkobject>
      </hkparam>
      <hkparam name="referencePose" numelements="3">
        (0.000000 0.000000 0.000000)(0.000000 0.000000 0.000000 1.000000)(1.000000 1.000000 1.000000)
        (1.000000 0.000000 0.000000)(0.000000 0.000000 0.000000 1.000000)(1.000000 1.000000 1.000000)
        (0.000000 2.000000 0.000000)(0.000000 0.000000 0.000000 1.000000)(1.000000 1.000000 1.000000)
      </hkparam>
      <hkparam name="floatSlots" numelements="1">
        <hkcstring>MorphWeight</hkcstring>
      </hkparam>
      <hkparam name="partitions" numelements="1">
        <hkobject>
          <hkparam name="name">Body</hkparam>
          <hkparam name="startBoneIndex">0</hkparam>
          <hkparam name="numBones">3</hkparam>
        </hkobject>
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>
"""


class TestParseSkeleton:
    def test_extracts_bone_names(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        assert result.bone_names == ["Root", "Bone01", "Bone02"]

    def test_extracts_parent_indices(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        assert result.parent_indices == [-1, 0, 1]

    def test_bone_count(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        assert result.bone_count == 3

    def test_extracts_reference_pose(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        assert len(result.reference_pose) == 3
        # Each pose entry: {"t": [x,y,z], "q": [x,y,z,w], "s": [x,y,z]}
        assert result.reference_pose[0]["t"] == [0.0, 0.0, 0.0]
        assert result.reference_pose[1]["t"] == [1.0, 0.0, 0.0]

    def test_extracts_float_slots(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        assert result.float_slots == ["MorphWeight"]

    def test_extracts_partition_names(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        assert result.partition_names == ["Body"]

    def test_serializes_to_json(self, tmp_path):
        xml_path = tmp_path / "skeleton.xml"
        xml_path.write_text(SAMPLE_SKELETON_XML)
        result = parse_skeleton(xml_path)
        # Verify JSON-serializable fields
        json.dumps(result.bone_names)
        json.dumps(result.parent_indices)
        json.dumps(result.reference_pose)
