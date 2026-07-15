import pytest
import json
from pathlib import Path


@pytest.fixture
def project_root():
    return Path(__file__).resolve().parents[4]


def test_rig_data_compat_with_index_skeleton(project_root):
    """RigData should be usable with index_skeleton."""
    from creation_lib.starfield_anim.rig_reader import parse_rig

    rig_path = (
        project_root
        / "extracted/starfield/meshes/actors/ballisticturret/characterassets/skeleton.rig"
    )
    if not rig_path.exists():
        pytest.skip("Starfield extracted data not available")
    data = parse_rig(rig_path)
    # These attributes must exist and be JSON-serializable
    assert hasattr(data, "bone_count")
    assert hasattr(data, "bone_names")
    assert hasattr(data, "parent_indices")
    assert hasattr(data, "reference_pose")
    json.dumps(data.bone_names)
    json.dumps(data.parent_indices)
    json.dumps(data.reference_pose)
    # Must also have float_count and partition_names (can be defaults)
    assert hasattr(data, "float_count")
    assert hasattr(data, "partition_names")


def test_af_data_compat_with_index_animation(project_root):
    """AfData should be usable with index_animation."""
    from creation_lib.starfield_anim.af_reader import parse_af

    af_path = (
        project_root
        / "extracted/starfield/meshes/actors/ballisticturret/animations/idle.af"
    )
    if not af_path.exists():
        pytest.skip("Starfield extracted data not available")
    data = parse_af(af_path)
    assert hasattr(data, "compression_type")
    assert hasattr(data, "bone_count")
    assert hasattr(data, "duration")
    assert hasattr(data, "frame_count")
    assert hasattr(data, "annotation_tracks")
    assert hasattr(data, "frame0_transforms")
    json.dumps(data.annotation_tracks)


def test_agx_data_compat_with_index_behavior(project_root):
    """AgxData should be usable with index_behavior and build_fts_content."""
    from creation_lib.starfield_anim.agx_reader import parse_agx
    from creation_lib.preprocessor.havok import build_fts_content

    agx_path = (
        project_root
        / "extracted/starfield/meshes/animtextdata/tables/graphs/simpleidleloop01.agx"
    )
    if not agx_path.exists():
        pytest.skip("Starfield extracted data not available")
    data = parse_agx(agx_path)
    assert hasattr(data, "node_count")
    assert hasattr(data, "events")
    assert hasattr(data, "variables")
    assert hasattr(data, "sequences")
    assert hasattr(data, "transitions")
    assert hasattr(data, "node_classes")
    # build_fts_content should not crash
    content = build_fts_content("test", data, "test/path", "test/graph")
    assert isinstance(content, str)
