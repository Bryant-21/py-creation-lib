from creation_lib.havok.parsers.project import ProjectData, parse_project
from creation_lib.havok.parsers.character import CharacterData, parse_character


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


class TestParseProject:
    def test_extracts_character_filenames(self, tmp_path):
        xml_path = tmp_path / "Project.xml"
        xml_path.write_text(SAMPLE_PROJECT_XML)
        result = parse_project(xml_path)
        assert result.character_filenames == ["Characters\\Character.hkx"]

    def test_handles_empty_project(self, tmp_path):
        xml_path = tmp_path / "Empty.xml"
        xml_path.write_text('<?xml version="1.0"?><hkpackfile><hksection name="__data__"></hksection></hkpackfile>')
        result = parse_project(xml_path)
        assert result.character_filenames == []


class TestParseCharacter:
    def test_extracts_rig_name(self, tmp_path):
        xml_path = tmp_path / "Character.xml"
        xml_path.write_text(SAMPLE_CHARACTER_XML)
        result = parse_character(xml_path)
        assert "SingleBoneSkeleton.hkt" in result.rig_name

    def test_extracts_behavior_filename(self, tmp_path):
        xml_path = tmp_path / "Character.xml"
        xml_path.write_text(SAMPLE_CHARACTER_XML)
        result = parse_character(xml_path)
        assert result.behavior_filename == "Behaviors\\Behavior.hkx"

    def test_handles_missing_string_data(self, tmp_path):
        xml_path = tmp_path / "Minimal.xml"
        xml_path.write_text('<?xml version="1.0"?><hkpackfile><hksection name="__data__"></hksection></hkpackfile>')
        result = parse_character(xml_path)
        assert result.rig_name == ""
        assert result.behavior_filename == ""
