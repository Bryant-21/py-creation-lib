from pathlib import Path
import pytest


@pytest.fixture
def repo_root() -> Path:
    # tests/ → editor/ → esp/ → creation_lib/ → python/ → py_creation_lib/ → repo root
    return Path(__file__).resolve().parents[6]
