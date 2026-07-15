from __future__ import annotations

from creation_lib.animation.models import (
    AnimationClip,
    AnimationEvent,
    AnimationKeyframe,
    BoneChannel,
    FloatChannel,
)
from creation_lib.max.kf_bridge import (
    animation_document_from_clip,
    clip_from_animation_document,
)


def test_kf_bridge_preserves_transform_channel():
    clip = AnimationClip(
        name="Idle",
        duration=1.0,
        frequency=1.0,
        cycle_type="clamp",
        channels=(
            BoneChannel(
                bone_name="Root",
                priority=26,
                translations=(
                    AnimationKeyframe(time=0.0, value=(1.0, 2.0, 3.0)),
                ),
                rotations=(
                    AnimationKeyframe(time=0.0, value=(0.0, 0.0, 0.0, 1.0)),
                ),
                scales=(AnimationKeyframe(time=0.0, value=(1.0,)),),
            ),
        ),
        float_channels=(
            FloatChannel(
                target_name="Head",
                property_type="morph",
                controller_type="NiFloatInterpolator",
                keyframes=(AnimationKeyframe(time=0.5, value=(0.75,)),),
            ),
        ),
        events=(AnimationEvent(time=0.25, text="FootLeft"),),
        warnings=("sample warning",),
    )

    document = animation_document_from_clip(clip, source_path="idle.kf")
    round_trip = clip_from_animation_document(document)

    assert document["kind"] == "kf_animation_document"
    assert document["source_path"] == "idle.kf"
    assert round_trip == clip


def test_kf_bridge_normalizes_max_cycle_types():
    clip = clip_from_animation_document(
        {
            "name": "Idle",
            "duration": 1.0,
            "cycle_type": "CYCLE_LOOP",
        }
    )

    document = animation_document_from_clip(
        AnimationClip(name="Idle", duration=1.0, cycle_type="CYCLE_CLAMP")
    )

    assert clip.cycle_type == "loop"
    assert document["cycle_type"] == "clamp"
