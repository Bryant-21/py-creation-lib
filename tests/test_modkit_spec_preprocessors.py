from pathlib import Path

from creation_lib.preprocessor.preprocess_runner import _PREPROCESS_MODULES


def test_modkit_spec_includes_dynamic_preprocessor_imports():
    spec = Path("modkit.spec").read_text(encoding="utf-8")

    for module_name in set(_PREPROCESS_MODULES.values()):
        assert f'"{module_name}"' in spec
