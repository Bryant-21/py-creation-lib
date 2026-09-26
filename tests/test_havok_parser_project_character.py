from creation_lib.havok.parsers.project import parse_project
from creation_lib.havok.parsers.character import parse_character


SAMPLE_PROJECT_XML = """\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbProjectData" signature="0x13a39ba7">
      <hkparam name="worldUpWS">(0.000000 0.000000 1.000000 0.000000)</hkparam>
    </hkobject>
    <hkobject name="#0002" class="hkbProjectStringData" signature="0x76ad60a">
      <hkparam name="characterFilenames" numelements="1">
        <hkcstring>Characters\\Character.hkx</hkcstring>
      </hkparam>
    </hkobject>
  </hksection>
</hkpackfile>
"""

SAMPLE_CHARACTER_XML = """\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbCharacterData" signature="0x300d6808">
      <hkparam name="modelUpMS">(0.000000 0.000000 1.000000 0.000000)</hkparam>
      <hkparam name="modelForwardMS">(0.000000 1.000000 0.000000 0.000000)</hkparam>
      <hkparam name="modelRightMS">(1.000000 0.000000 0.000000 0.000000)</hkparam>
    </hkobject>
    <hkobject name="#0002" class="hkbCharacterStringData" signature="0x655b42bc">
      <hkparam name="rigName">..\\..\\GenericBehaviors\\zSingleBoneSkeleton\\SingleBoneSkeleton.hkt</hkparam>
      <hkparam name="behaviorFilename">Behaviors\\Behavior.hkx</hkparam>
    </hkobject>
  </hksection>
</hkpackfile>
"""


def test_parse_project_extracts_character_filenames_and_handles_empty(tmp_path):
    xml_path = tmp_path / "Project.xml"
    xml_path.write_text(SAMPLE_PROJECT_XML)
    result = parse_project(xml_path)
    assert result.character_filenames == ["Characters\\Character.hkx"]

    empty_path = tmp_path / "Empty.xml"
    empty_path.write_text('<?xml version="1.0"?><hkpackfile><hksection name="__data__"></hksection></hkpackfile>')
    assert parse_project(empty_path).character_filenames == []


def test_parse_character_extracts_rig_and_behavior_and_handles_missing_data(tmp_path):
    xml_path = tmp_path / "Character.xml"
    xml_path.write_text(SAMPLE_CHARACTER_XML)
    result = parse_character(xml_path)
    assert "SingleBoneSkeleton.hkt" in result.rig_name
    assert result.behavior_filename == "Behaviors\\Behavior.hkx"

    minimal_path = tmp_path / "Minimal.xml"
    minimal_path.write_text('<?xml version="1.0"?><hkpackfile><hksection name="__data__"></hksection></hkpackfile>')
    result = parse_character(minimal_path)
    assert result.rig_name == ""
    assert result.behavior_filename == ""
