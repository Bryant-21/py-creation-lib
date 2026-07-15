import os

import pytest
from pathlib import Path
from creation_lib.starfield_anim.af_reader import parse_af, AfData

EMPTY_AF = Path("extracted/starfield/meshes/actors/critter/animations/meleeattack03.af")
TURRET_IDLE_AF = Path("extracted/starfield/meshes/actors/ballisticturret/animations/idle.af")


@pytest.fixture
def project_root():
    return Path(__file__).resolve().parents[4]


def test_parse_empty_af(project_root):
    """64-byte minimal .af should parse without error (0 bones)."""
    af_path = project_root / EMPTY_AF
    if not af_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_af(af_path)
    assert isinstance(result, AfData)
    assert result.bone_count == 0
    assert result.version == 5


def test_parse_turret_idle(project_root):
    """Turret idle .af should have bones and frames."""
    af_path = project_root / TURRET_IDLE_AF
    if not af_path.exists():
        pytest.skip("Starfield extracted data not available")
    result = parse_af(af_path)
    assert result.bone_count > 0
    assert result.frame_count > 0
    assert result.duration > 0.0
    # Flags should be a valid integer
    assert isinstance(result.flags, int)


def test_parse_all_af_files_no_errors(project_root):
    """Parse a sample of .af files and verify no exceptions."""
    af_dir = project_root / "extracted/starfield/meshes/actors"
    if not af_dir.exists():
        pytest.skip("Starfield extracted data not available")
    count = 0
    errors = []
    for root_dir, _, files in os.walk(str(af_dir)):
        for f in files:
            if not f.endswith(".af"):
                continue
            af_path = Path(root_dir) / f
            try:
                result = parse_af(af_path)
                assert result.version >= 0
                count += 1
            except Exception as e:
                errors.append(f"{af_path}: {e}")
            if count >= 500:  # Sample first 500 for speed
                break
        if count >= 500:
            break
    assert len(errors) == 0, f"Parse errors:\n" + "\n".join(errors[:10])
    assert count > 0


def test_parse_nonexistent_af():
    """Nonexistent file should return empty AfData."""
    result = parse_af(Path("/nonexistent/file.af"))
    assert result.bone_count == 0
