import pytest
from pathlib import Path
from creation_lib.starfield_anim.agx_reader import parse_agx, AgxData

SIMPLE_AGX = Path("extracted/starfield/meshes/animtextdata/tables/graphs/simpleidleloop01.agx")

@pytest.fixture
def project_root():
    return Path(__file__).resolve().parents[4]

def test_parse_simple_agx(project_root):
    """SimpleIdleLoop01 should parse with name, category, and nodes."""
    agx_path = project_root / SIMPLE_AGX
    if not agx_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_agx(agx_path)
    assert isinstance(result, AgxData)
    assert "SimpleIdleLoop01" in result.name
    assert result.category != ""
    assert result.node_count > 0
    assert len(result.node_classes) > 0
    # Variables and events should be lists (may be empty for simple graphs)
    assert isinstance(result.variables, list)
    assert isinstance(result.events, list)
    assert isinstance(result.sequences, list)
    # Variables should be list of (name, type) tuples
    for v in result.variables:
        assert isinstance(v, tuple) and len(v) == 2


LOCOMOTION_AGX = Path("extracted/starfield/meshes/animtextdata/tables/graphs/biped_idlelocomotion.agx")

def test_parse_locomotion_agx(project_root):
    """biped_idlelocomotion should have multiple node types."""
    agx_path = project_root / LOCOMOTION_AGX
    if not agx_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_agx(agx_path)
    assert result.node_count > 1
    assert len(result.node_classes) > 1
