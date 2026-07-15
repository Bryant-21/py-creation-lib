from creation_lib.preprocessor.preprocess_runner import resolve_preprocess_module, run_preprocess


def test_resolve_behaviors_alias_to_havok():
    assert (
        resolve_preprocess_module("preprocess_behaviors.py") == "creation_lib.preprocessor.havok"
    )


def test_run_preprocess_reports_invalid_game_lines():
    lines = []

    rc = run_preprocess(
        "preprocess_nifs.py",
        "--game",
        "notagame",
        on_line=lines.append,
    )

    assert rc == 1
    assert any("Invalid --game 'notagame'" in line for line in lines)
