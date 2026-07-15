import pytest
from pathlib import Path
from creation_lib.starfield_anim.rig_reader import parse_rig, RigData

# 1-bone generic rig (495 bytes)
GENERIC_RIG = Path("extracted/starfield/meshes/genericbehaviors/characterassets/skeleton.rig")

TURRET_RIG = Path("extracted/starfield/meshes/actors/ballisticturret/characterassets/skeleton.rig")
HUMAN_RIG = Path("extracted/starfield/meshes/actors/human/characterassets/skeleton.rig")

@pytest.fixture
def project_root():
    """Return project root (3 levels up from this test file)."""
    return Path(__file__).resolve().parents[4]

def test_parse_generic_rig_basic(project_root):
    """Single-bone rig should parse with 1 bone named 'Root'."""
    rig_path = project_root / GENERIC_RIG
    if not rig_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_rig(rig_path)
    assert isinstance(result, RigData)
    assert result.version == 5
    assert result.bone_count == 1
    assert result.bone_names == ["Root"]
    assert result.parent_indices == [-1]  # Root has no parent
    assert len(result.reference_pose) == 1
    # Pose should be normalized to {t, q, s} format matching SkeletonData
    pose = result.reference_pose[0]
    assert "t" in pose and "q" in pose and "s" in pose
    assert len(pose["q"]) == 4  # xyzw quaternion
    assert pose["s"] == [1.0, 1.0, 1.0]  # scale always 1

def test_parse_turret_rig(project_root):
    """Turret rig: 7 bones with known hierarchy."""
    rig_path = project_root / TURRET_RIG
    if not rig_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_rig(rig_path)
    assert result.bone_count == 7
    assert result.animated_bone_count <= result.bone_count
    assert len(result.bone_names) == 7
    assert "Root" in result.bone_names
    assert "COM" in result.bone_names
    assert len(result.parent_indices) == 7
    assert result.parent_indices[0] == -1  # Root
    assert len(result.bone_map) == 157
    assert result.low_precision > 0
    assert result.high_precision > 0

def test_parse_human_rig(project_root):
    """Human rig: ~97 bones with twist bones."""
    rig_path = project_root / HUMAN_RIG
    if not rig_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_rig(rig_path)
    assert result.bone_count > 80  # Human has ~97
    assert "C_Hips" in result.bone_names
    assert "C_Spine" in result.bone_names
    assert "C_Head" in result.bone_names
    # Should have twist bones
    has_twist = any(bt == 1 for bt in result.bone_types)
    assert has_twist, "Human rig should have twist bones"
    # Reference pose should have correct count
    assert len(result.reference_pose) == result.bone_count

def test_parse_nonexistent_rig():
    """Nonexistent file should return empty RigData."""
    result = parse_rig(Path("/nonexistent/file.rig"))
    assert result.bone_count == 0
    assert result.bone_names == []
