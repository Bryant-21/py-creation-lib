"""Smoke test: verify Slice 10 pyfunctions exist on havok_native and accept inputs."""

import pytest

SKELETON_XML = """<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#skeleton" class="hkaSkeleton" signature="0x366e8220">
            <hkparam name="name">TestSkeleton</hkparam>
            <hkparam name="parentIndices" numelements="2">-1 0</hkparam>
            <hkparam name="bones" numelements="2">
                <hkobject><hkparam name="name">Root</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
                <hkobject><hkparam name="name">Spine</hkparam><hkparam name="lockTranslation">0</hkparam></hkobject>
            </hkparam>
        </hkobject>
    </hksection>
</hkpackfile>"""

ANIMATION_XML = """<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#animation" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
            <hkparam name="duration">1.000000</hkparam>
            <hkparam name="numberOfTransformTracks">2</hkparam>
            <hkparam name="numberOfFloatTracks">0</hkparam>
            <hkparam name="transforms" numelements="4">(0 0 0 0 0 0 0 1 1 1 1 0)(1 0 0 0 0 0 0 1 1 1 1 0)(2 0 0 0 0 0 0 1 1 1 1 0)(3 0 0 0 0 0 0 1 1 1 1 0)</hkparam>
        </hkobject>
    </hksection>
</hkpackfile>"""

BEHAVIOR_XML = """<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#data" class="hkbBehaviorGraphStringData" signature="0x6d26f61d">
            <hkparam name="eventNames" numelements="1"><hkcstring>Footstep</hkcstring></hkparam>
            <hkparam name="variableNames" numelements="0"></hkparam>
        </hkobject>
    </hksection>
</hkpackfile>"""


def test_slice10_pyfunctions_exist_and_accept_inputs():
    import json
    from creation_lib.havok.native_runtime import load_native_module

    m = load_native_module()

    # All five new pyfunctions must be present
    for name in [
        "havok_extract_clip",
        "havok_write_animation_xml",
        "havok_collision_preview",
        "havok_parse_skeleton",
        "havok_parse_behavior",
    ]:
        assert hasattr(m, name), f"missing pyfunction: {name}"

    # havok_extract_clip returns JSON with 'channels'
    clip_json = m.havok_extract_clip(ANIMATION_XML, None)
    data = json.loads(clip_json)
    assert "channels" in data

    # havok_write_animation_xml round-trips back to XML
    xml_out = m.havok_write_animation_xml(clip_json, ["Root", "Spine"])
    assert "hkaInterleavedUncompressedAnimation" in xml_out

    # havok_parse_skeleton returns JSON with bone_names
    skel_json = m.havok_parse_skeleton(SKELETON_XML)
    skel = json.loads(skel_json)
    assert "bone_names" in skel
    assert "Root" in skel["bone_names"]

    # havok_parse_behavior returns JSON with events
    beh_json = m.havok_parse_behavior(BEHAVIOR_XML)
    beh = json.loads(beh_json)
    assert "events" in beh
    assert "Footstep" in beh["events"]

    # havok_collision_preview accepts empty bytes without panicking
    preview_json = m.havok_collision_preview(b"", 1.0)
    preview = json.loads(preview_json)
    assert "meshes" in preview


def test_tagxml_reader_accepts_named_object_pointers(tmp_path):
    from creation_lib.havok.native_runtime import load_native_module

    xml = """<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#container" class="hkaAnimationContainer" signature="0x8dc20f3">
      <hkparam name="animations" numelements="1">#animation</hkparam>
      <hkparam name="bindings" numelements="1">#binding</hkparam>
    </hkobject>
    <hkobject name="#animation" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
      <hkparam name="duration">0.0</hkparam>
      <hkparam name="numberOfTransformTracks">0</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="transforms" numelements="0"></hkparam>
    </hkobject>
    <hkobject name="#binding" class="hkaAnimationBinding" signature="0x66eac971">
      <hkparam name="animation">#animation</hkparam>
      <hkparam name="transformTrackToBoneIndices" numelements="0"></hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"""
    native = load_native_module()
    xml_path = tmp_path / "named_pointers.xml"
    hkx_path = tmp_path / "named_pointers.hkx"
    xml_path.write_text(xml, encoding="ascii")

    native.pack_xml_to_hkx(str(xml_path), str(hkx_path))
    assert "hkaAnimationContainer" in native.unpack_hkx_to_xml(str(hkx_path))


def test_tagxml_reader_rejects_duplicate_hkobject_names(tmp_path):
    from creation_lib.havok.native_runtime import load_native_module

    xml = """<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
  <hksection name="__data__">
    <hkobject name="#container" class="hkaAnimationContainer" signature="0x8dc20f3">
      <hkparam name="animations" numelements="1">#animation</hkparam>
    </hkobject>
    <hkobject name="#animation" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
      <hkparam name="duration">0.0</hkparam>
      <hkparam name="numberOfTransformTracks">0</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="transforms" numelements="0"></hkparam>
    </hkobject>
    <hkobject name="#animation" class="hkaInterleavedUncompressedAnimation" signature="0x930af031">
      <hkparam name="duration">0.0</hkparam>
      <hkparam name="numberOfTransformTracks">0</hkparam>
      <hkparam name="numberOfFloatTracks">0</hkparam>
      <hkparam name="transforms" numelements="0"></hkparam>
    </hkobject>
  </hksection>
</hkpackfile>"""
    native = load_native_module()
    xml_path = tmp_path / "duplicate_names.xml"
    hkx_path = tmp_path / "duplicate_names.hkx"
    xml_path.write_text(xml, encoding="ascii")

    with pytest.raises(ValueError, match="duplicate hkobject name #animation"):
        native.pack_xml_to_hkx(str(xml_path), str(hkx_path))
