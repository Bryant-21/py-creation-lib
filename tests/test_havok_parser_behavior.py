"""Tests for creation_lib.havok.parsers — Havok XML file parsers."""
import tempfile
from pathlib import Path

import pytest

from creation_lib.havok.parsers.behavior import BehaviorData, parse_behavior


# Minimal valid behavior XML with events, variables, sequences, transitions.
SAMPLE_BEHAVIOR_XML = """\
<?xml version="1.0" encoding="ascii"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#0001" class="hkbBehaviorGraphStringData" signature="0xc713064e">
      <hkparam name="eventNames" numelements="2">
        <hkcstring>EquipWeapon</hkcstring>
        <hkcstring>UnequipWeapon</hkcstring>
      </hkparam>
      <hkparam name="variableNames" numelements="1">
        <hkcstring>Speed</hkcstring>
      </hkparam>
    </hkobject>
    <hkobject name="#0002" class="hkbBehaviorGraphData" signature="0x95aca5d">
      <hkparam name="variableInfos" numelements="1">
        <hkobject>
          <hkparam name="type">VARIABLE_TYPE_REAL</hkparam>
        </hkobject>
      </hkparam>
    </hkobject>
    <hkobject name="#0003" class="BGSGamebryoSequenceGenerator" signature="0xbee5cafe">
      <hkparam name="pSequence">IdleLoop</hkparam>
    </hkobject>
    <hkobject name="#0004" class="hkbBlendingTransitionEffect" signature="0xfd8584fe">
      <hkparam name="name">FadeIn</hkparam>
      <hkparam name="duration">0.300000</hkparam>
    </hkobject>
    <hkobject name="#0005" class="hkbStateMachineStateInfo" signature="0x0ed7f9d0">
    </hkobject>
  </hksection>
</hkpackfile>
"""


class TestParseBehavior:
    def test_extracts_events(self, tmp_path):
        xml_path = tmp_path / "Behavior.xml"
        xml_path.write_text(SAMPLE_BEHAVIOR_XML)
        result = parse_behavior(xml_path)
        assert result.events == ["EquipWeapon", "UnequipWeapon"]

    def test_extracts_variables(self, tmp_path):
        xml_path = tmp_path / "Behavior.xml"
        xml_path.write_text(SAMPLE_BEHAVIOR_XML)
        result = parse_behavior(xml_path)
        assert result.variables == [("Speed", "VARIABLE_TYPE_REAL")]

    def test_extracts_sequences(self, tmp_path):
        xml_path = tmp_path / "Behavior.xml"
        xml_path.write_text(SAMPLE_BEHAVIOR_XML)
        result = parse_behavior(xml_path)
        assert result.sequences == ["IdleLoop"]

    def test_extracts_transitions(self, tmp_path):
        xml_path = tmp_path / "Behavior.xml"
        xml_path.write_text(SAMPLE_BEHAVIOR_XML)
        result = parse_behavior(xml_path)
        assert result.transitions == [("FadeIn", "0.300000")]

    def test_counts_nodes(self, tmp_path):
        xml_path = tmp_path / "Behavior.xml"
        xml_path.write_text(SAMPLE_BEHAVIOR_XML)
        result = parse_behavior(xml_path)
        assert result.node_count == 5

    def test_collects_node_classes(self, tmp_path):
        xml_path = tmp_path / "Behavior.xml"
        xml_path.write_text(SAMPLE_BEHAVIOR_XML)
        result = parse_behavior(xml_path)
        assert "hkbBehaviorGraphStringData" in result.node_classes
        assert "BGSGamebryoSequenceGenerator" in result.node_classes

    def test_handles_empty_xml(self, tmp_path):
        xml_path = tmp_path / "Empty.xml"
        xml_path.write_text('<?xml version="1.0"?><hkpackfile><hksection name="__data__"></hksection></hkpackfile>')
        result = parse_behavior(xml_path)
        assert result.events == []
        assert result.variables == []
        assert result.node_count == 0

    def test_handles_malformed_xml(self, tmp_path):
        xml_path = tmp_path / "Bad.xml"
        xml_path.write_text("not xml at all")
        result = parse_behavior(xml_path)
        assert result.events == []
        assert result.node_count == 0
