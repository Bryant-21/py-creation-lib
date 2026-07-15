"""Tests for the Havok animation XML writer."""
from __future__ import annotations

import dataclasses
import math
import xml.etree.ElementTree as ET
from pathlib import Path
from tempfile import TemporaryDirectory

import pytest

from creation_lib.animation.models import (
    AnimationClip,
    AnimationEvent,
    AnimationKeyframe,
    BoneChannel,
)
from creation_lib.havok.animation_writer import (
    SAMPLE_RATE,
    _lerp_tuple,
    _slerp,
    write_animation_xml,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_clip() -> AnimationClip:
    """Create a simple 2-bone clip with a few keyframes and 2 events."""
    ch_pelvis = BoneChannel(
        bone_name="Pelvis",
        translations=(
            AnimationKeyframe(time=0.0, value=(0.0, 0.0, 0.0)),
            AnimationKeyframe(time=1.0, value=(10.0, 0.0, 0.0)),
        ),
        rotations=(
            AnimationKeyframe(time=0.0, value=(0.0, 0.0, 0.0, 1.0)),
            AnimationKeyframe(time=1.0, value=(0.0, 0.707107, 0.0, 0.707107)),
        ),
        scales=(
            AnimationKeyframe(time=0.0, value=(1.0, 1.0, 1.0)),
        ),
    )
    ch_spine = BoneChannel(
        bone_name="Spine",
        rotations=(
            AnimationKeyframe(time=0.0, value=(0.0, 0.0, 0.0, 1.0)),
            AnimationKeyframe(time=0.5, value=(0.0, 0.0, 0.383, 0.924)),
            AnimationKeyframe(time=1.0, value=(0.0, 0.0, 0.0, 1.0)),
        ),
    )
    return AnimationClip(
        name="test_anim",
        duration=1.0,
        channels=(ch_pelvis, ch_spine),
        events=(
            AnimationEvent(time=0.25, text="FootLeft"),
            AnimationEvent(time=0.75, text="FootRight"),
        ),
    )


def _parse_xml(path: Path) -> ET.ElementTree:
    return ET.parse(str(path))


# ---------------------------------------------------------------------------
# Tests: quaternion slerp
# ---------------------------------------------------------------------------

class TestSlerp:
    def test_identity(self):
        q = (0.0, 0.0, 0.0, 1.0)
        result = _slerp(q, q, 0.5)
        assert all(abs(a - b) < 1e-6 for a, b in zip(result, q))

    def test_endpoints(self):
        q0 = (0.0, 0.0, 0.0, 1.0)
        q1 = (0.0, 0.707107, 0.0, 0.707107)
        r0 = _slerp(q0, q1, 0.0)
        r1 = _slerp(q0, q1, 1.0)
        assert all(abs(a - b) < 1e-5 for a, b in zip(r0, q0))
        assert all(abs(a - b) < 1e-5 for a, b in zip(r1, q1))

    def test_midpoint_normalized(self):
        q0 = (0.0, 0.0, 0.0, 1.0)
        q1 = (0.0, 1.0, 0.0, 0.0)
        mid = _slerp(q0, q1, 0.5)
        length = math.sqrt(sum(x * x for x in mid))
        assert abs(length - 1.0) < 1e-6


# ---------------------------------------------------------------------------
# Tests: lerp
# ---------------------------------------------------------------------------

class TestLerp:
    def test_basic(self):
        a = (0.0, 0.0, 0.0)
        b = (10.0, 20.0, 30.0)
        r = _lerp_tuple(a, b, 0.5)
        assert r == pytest.approx((5.0, 10.0, 15.0))


# ---------------------------------------------------------------------------
# Tests: XML output
# ---------------------------------------------------------------------------

class TestWriteAnimationXml:
    def test_basic_structure(self, tmp_path: Path):
        clip = _make_clip()
        skeleton_bones = ["Pelvis", "Spine"]
        out = tmp_path / "test_anim.xml"

        write_animation_xml(clip, skeleton_bones, out)

        assert out.exists()
        tree = _parse_xml(out)
        root = tree.getroot()

        assert root.tag == "hkpackfile"
        assert root.get("contentsversion") == "hk_2014.1.0-r1"

        section = root.find("hksection")
        assert section is not None
        assert section.get("name") == "__data__"

    def test_animation_container(self, tmp_path: Path):
        clip = _make_clip()
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, ["Pelvis", "Spine"], out)

        tree = _parse_xml(out)
        objects = tree.getroot().findall(".//hkobject")
        container = [o for o in objects if o.get("class") == "hkaAnimationContainer"]
        assert len(container) == 1

        anims_param = container[0].find("hkparam[@name='animations']")
        assert anims_param is not None
        assert anims_param.get("numelements") == "1"
        assert "#animation" in anims_param.text

    def test_transform_count(self, tmp_path: Path):
        clip = _make_clip()
        bones = ["Pelvis", "Spine"]
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, bones, out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]

        # duration=1.0 at 30fps => 31 frames, 2 bones => 62 transforms
        expected_frames = int(1.0 * SAMPLE_RATE) + 1
        expected_transforms = expected_frames * len(bones)

        transforms_param = anim_obj.find("hkparam[@name='transforms']")
        assert transforms_param.get("numelements") == str(expected_transforms)

        # Count actual transform entries. The native writer emits the canonical
        # packed hkQsTransform form: one 12-float tuple per transform.
        import re
        groups = re.findall(r"\([^)]+\)", transforms_param.text)
        assert len(groups) == expected_transforms
        assert len(groups[0].strip("()").split()) == 12

    def test_frame_count_at_30fps(self, tmp_path: Path):
        """Verify frame count formula: int(duration * 30) + 1."""
        clip = AnimationClip(
            name="short",
            duration=0.5,
            channels=(),
            events=(),
        )
        bones = ["Root"]
        out = tmp_path / "short.xml"
        write_animation_xml(clip, bones, out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]

        expected_frames = int(0.5 * 30) + 1  # 16
        transforms_param = anim_obj.find("hkparam[@name='transforms']")
        assert transforms_param.get("numelements") == str(expected_frames)

    def test_annotation_events(self, tmp_path: Path):
        clip = _make_clip()
        bones = ["Pelvis", "Spine"]
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, bones, out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]

        ann_param = anim_obj.find("hkparam[@name='annotationTracks']")
        assert ann_param is not None
        assert ann_param.get("numelements") == "2"  # 2 bones

        # Events are on track 0 (Pelvis)
        track_objects = ann_param.findall("hkobject")
        assert len(track_objects) == 2

        track0_anns = track_objects[0].find("hkparam[@name='annotations']")
        assert track0_anns.get("numelements") == "2"

        # Track 1 (Spine) has no events
        track1_anns = track_objects[1].find("hkparam[@name='annotations']")
        assert track1_anns.get("numelements") == "0"

    def test_binding(self, tmp_path: Path):
        clip = _make_clip()
        bones = ["Pelvis", "Spine"]
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, bones, out)

        tree = _parse_xml(out)
        binding_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaAnimationBinding"
        ][0]

        indices_param = binding_obj.find(
            "hkparam[@name='transformTrackToBoneIndices']"
        )
        assert indices_param.get("numelements") == "2"
        assert indices_param.text.strip() == "0 1"

    def test_missing_bone_gets_identity(self, tmp_path: Path):
        """Bones in skeleton but not in clip get identity transforms."""
        clip = _make_clip()  # has Pelvis and Spine
        bones = ["Pelvis", "Spine", "Head"]  # Head not in clip
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, bones, out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]

        transforms_param = anim_obj.find("hkparam[@name='transforms']")
        import re
        groups = re.findall(r"\([^)]+\)", transforms_param.text)

        # 31 frames * 3 bones, one packed hkQsTransform tuple per transform.
        assert len(groups) == 31 * 3

        # Check that frame 0 bone 2 (Head) is identity
        # Frame 0: bones 0,1,2 => group index 2 for Head
        head_values = [float(value) for value in groups[2].strip("()").split()]
        assert head_values[:3] == pytest.approx([0.0, 0.0, 0.0])
        assert head_values[4:8] == pytest.approx([0.0, 0.0, 0.0, 1.0])
        assert head_values[8:11] == pytest.approx([1.0, 1.0, 1.0])

    def test_zero_duration(self, tmp_path: Path):
        """Zero-duration clip produces exactly 1 frame."""
        clip = AnimationClip(name="still", duration=0.0, channels=())
        out = tmp_path / "still.xml"
        write_animation_xml(clip, ["Root"], out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]

        transforms_param = anim_obj.find("hkparam[@name='transforms']")
        assert transforms_param.get("numelements") == "1"


class TestPreservesAdditionalFields:
    """Writer must round-trip extractedMotion / blendHint / track-bone binding."""

    def test_writer_preserves_extracted_motion(self, tmp_path: Path):
        clip = dataclasses.replace(_make_clip(), extracted_motion_ref="#0006")
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, ["Pelvis", "Spine"], out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]
        em = anim_obj.find("hkparam[@name='extractedMotion']")
        assert em is not None and em.text.strip() == "#0006", (
            f"expected extractedMotion=#0006, got {em.text!r}"
        )

    def test_writer_preserves_additive_blend_hint(self, tmp_path: Path):
        clip = dataclasses.replace(_make_clip(), is_additive=True)
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, ["Pelvis", "Spine"], out)

        tree = _parse_xml(out)
        binding_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaAnimationBinding"
        ][0]
        hint = binding_obj.find("hkparam[@name='blendHint']")
        assert hint is not None and hint.text.strip() == "ADDITIVE", (
            f"expected blendHint=ADDITIVE, got {hint.text!r}"
        )

    def test_writer_preserves_non_identity_track_to_bone(self, tmp_path: Path):
        # 2 tracks (Pelvis, Spine) rebound to non-identity skeleton slots.
        clip = dataclasses.replace(_make_clip(), track_to_bone_indices=(3, 7))
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, ["Pelvis", "Spine"], out)

        tree = _parse_xml(out)
        binding_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaAnimationBinding"
        ][0]
        idx = binding_obj.find("hkparam[@name='transformTrackToBoneIndices']")
        assert idx is not None and idx.text.strip() == "3 7", (
            f"expected '3 7', got {idx.text!r}"
        )

    def test_defaults_unchanged_for_unset_clip(self, tmp_path: Path):
        # Sanity: a vanilla clip still produces #null / NORMAL / identity.
        clip = _make_clip()
        out = tmp_path / "anim.xml"
        write_animation_xml(clip, ["Pelvis", "Spine"], out)

        tree = _parse_xml(out)
        anim_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaInterleavedUncompressedAnimation"
        ][0]
        binding_obj = [
            o for o in tree.getroot().findall(".//hkobject")
            if o.get("class") == "hkaAnimationBinding"
        ][0]
        assert anim_obj.find("hkparam[@name='extractedMotion']").text.strip() == "#null"
        assert binding_obj.find("hkparam[@name='blendHint']").text.strip() == "NORMAL"
        assert (
            binding_obj.find("hkparam[@name='transformTrackToBoneIndices']").text.strip()
            == "0 1"
        )
