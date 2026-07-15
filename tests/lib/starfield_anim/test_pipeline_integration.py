import pytest
from pathlib import Path


@pytest.fixture
def project_root():
    return Path(__file__).resolve().parents[4]


def test_parse_worker_dispatches_rig(project_root):
    """_parse_worker should handle role='skeleton' with .rig file."""
    from creation_lib.preprocessor.havok import _parse_worker

    rig_path = (
        project_root
        / "extracted/starfield/meshes/actors/ballisticturret/characterassets/skeleton.rig"
    )
    if not rig_path.exists():
        pytest.skip("Starfield extracted data not available")
    rel_path, role, data, entity_id, category, error = _parse_worker(
        (
            "actors/ballisticturret/characterassets/skeleton.rig",
            str(rig_path),
            "skeleton",
            "Creature",
            "starfield/actors/ballisticturret/characterassets/skeleton",
        )
    )
    assert error is None, f"Parse error: {error}"
    assert data is not None
    assert data.bone_count == 7


def test_parse_worker_dispatches_af(project_root):
    """_parse_worker should handle role='animation' with .af file."""
    from creation_lib.preprocessor.havok import _parse_worker

    af_path = (
        project_root
        / "extracted/starfield/meshes/actors/ballisticturret/animations/idle.af"
    )
    if not af_path.exists():
        pytest.skip("Starfield extracted data not available")
    rel_path, role, data, entity_id, category, error = _parse_worker(
        (
            "actors/ballisticturret/animations/idle.af",
            str(af_path),
            "animation",
            "Creature",
            "starfield/actors/ballisticturret/animations/idle",
        )
    )
    assert error is None, f"Parse error: {error}"
    assert data is not None
    assert data.bone_count > 0


def test_parse_worker_dispatches_agx(project_root):
    """_parse_worker should handle role='behavior' with .agx file."""
    from creation_lib.preprocessor.havok import _parse_worker

    agx_path = (
        project_root
        / "extracted/starfield/meshes/animtextdata/tables/graphs/simpleidleloop01.agx"
    )
    if not agx_path.exists():
        pytest.skip("Starfield extracted data not available")
    rel_path, role, data, entity_id, category, error = _parse_worker(
        (
            "animtextdata/tables/graphs/simpleidleloop01.agx",
            str(agx_path),
            "behavior",
            "AnimGraph",
            "starfield/animtextdata/tables/graphs/simpleidleloop01",
        )
    )
    assert error is None, f"Parse error: {error}"
    assert data is not None
    assert data.node_count > 0
